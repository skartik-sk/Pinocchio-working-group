//! Circuit Breaker - Implementation A
//!
//! Implements:
//!   1. Global Kill Switch (Admin-Triggered via multi-sig DAO)
//!   2. Volume/Rate Limiters (Time-Based outflow tracking)

use pinocchio::{
    AccountView, Address, entrypoint, msg, ProgramResult,
};

// ---------- Instruction discriminators ----------

const INITIALIZE: u8 = 0;
const SET_PAUSE: u8 = 1;
const TRANSFER: u8 = 2;
const UPDATE_THRESHOLD: u8 = 3;

// ---------- Error codes ----------

const ERR_PAUSED: u32 = 6000;
const ERR_RATE_LIMIT_EXCEEDED: u32 = 6001;
const ERR_UNAUTHORIZED: u32 = 6002;
const ERR_INVALID_ACCOUNT: u32 = 6003;

// ---------- State layout (stored in the config PDA) ----------
// Byte 0:       paused (bool)
// Byte 1..33:   authority (full 32-byte pubkey)
// Byte 33..41:  max_outflow (u64)
// Byte 41..49:  window_start (u64 — slot timestamp)
// Byte 49..57:  current_outflow (u64)

const OFFSET_PAUSED: usize = 0;
const OFFSET_AUTHORITY: usize = 1;
const AUTHORITY_LEN: usize = 32;
const OFFSET_MAX_OUTFLOW: usize = OFFSET_AUTHORITY + AUTHORITY_LEN; // 33
const OFFSET_WINDOW_START: usize = OFFSET_MAX_OUTFLOW + 8; // 41
const OFFSET_CURRENT_OUTFLOW: usize = OFFSET_WINDOW_START + 8; // 49

// ---------- Helpers ----------

fn read_u64(data: &[u8], offset: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(buf)
}

fn write_u64(data: &mut [u8], offset: usize, val: u64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}

/// Verify that the given account's pubkey matches the stored full 32-byte authority.
fn verify_authority(account: &AccountView, data: &[u8]) -> bool {
    let stored_auth = &data[OFFSET_AUTHORITY..OFFSET_AUTHORITY + AUTHORITY_LEN];
    let caller_auth = account.key().as_ref();
    // Constant-time comparison to prevent timing attacks
    let mut diff: u8 = 0;
    for i in 0..AUTHORITY_LEN {
        diff |= stored_auth[i] ^ caller_auth[i];
    }
    diff == 0
}

entrypoint!(process_instruction);

pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let discriminator = instruction_data.first().copied().unwrap_or(255);

    match discriminator {
        INITIALIZE => process_initialize(program_id, accounts, instruction_data),
        SET_PAUSE => process_set_pause(program_id, accounts, instruction_data),
        TRANSFER => process_transfer(program_id, accounts, instruction_data),
        UPDATE_THRESHOLD => process_update_threshold(program_id, accounts, instruction_data),
        _ => {
            msg!("Error: invalid instruction discriminator");
            Err(ERR_INVALID_ACCOUNT.into())
        }
    }
}

/// Initialize the circuit breaker config PDA.
///
/// Accounts:
///   0 — [signer, writable] authority (admin / DAO multi-sig)
///   1 — [writable]         config PDA ( uninitialized )
///   2 —                     system program
///
/// Data: [discriminator (1B), max_outflow (8B)]
fn process_initialize(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let [authority, config, _system_program] = accounts else {
        return Err(ERR_INVALID_ACCOUNT.into());
    };

    // Authority must sign
    if !authority.is_signer() {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let mut data = config.try_borrow_mut_data()?;

    // Parse max_outflow from instruction data
    let max_outflow = if instruction_data.len() >= 9 {
        read_u64(instruction_data, 1)
    } else {
        1_000_000 // default 1M lamports
    };

    // Write initial state — store FULL 32-byte pubkey
    data[OFFSET_PAUSED] = 0; // not paused
    let auth_key = authority.key();
    data[OFFSET_AUTHORITY..OFFSET_AUTHORITY + AUTHORITY_LEN]
        .copy_from_slice(auth_key.as_ref());
    write_u64(&mut data, OFFSET_MAX_OUTFLOW, max_outflow);
    write_u64(&mut data, OFFSET_WINDOW_START, 0);
    write_u64(&mut data, OFFSET_CURRENT_OUTFLOW, 0);

    msg!("Circuit breaker initialized");

    Ok(())
}

/// Set the global pause flag (kill switch).
///
/// Accounts:
///   0 — [signer] authority
///   1 — [writable] config PDA
///
/// Data: [discriminator (1B), paused (1B: 0 or 1)]
fn process_set_pause(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let [authority, config] = accounts else {
        return Err(ERR_INVALID_ACCOUNT.into());
    };

    if !authority.is_signer() {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let data = config.try_borrow_mut_data()?;

    // Verify authority — full 32-byte comparison
    if !verify_authority(authority, &data) {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let paused = instruction_data.get(1).copied().unwrap_or(1);
    data[OFFSET_PAUSED] = paused;

    if paused == 1 {
        msg!("Circuit breaker ACTIVATED — all transfers paused");
    } else {
        msg!("Circuit breaker DEACTIVATED — transfers resumed");
    }

    Ok(())
}

/// Process a transfer through the circuit breaker.
///
/// Accounts:
///   0 — [signer, writable] sender
///   1 — [writable]         recipient
///   2 — [writable]         config PDA
///
/// Data: [discriminator (1B), amount (8B), current_slot (8B)]
fn process_transfer(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let [_sender, _recipient, config] = accounts else {
        return Err(ERR_INVALID_ACCOUNT.into());
    };

    let mut data = config.try_borrow_mut_data()?;

    // --- Check 1: Global kill switch ---
    if data[OFFSET_PAUSED] == 1 {
        msg!("Error: circuit breaker is PAUSED");
        return Err(ERR_PAUSED.into());
    }

    // --- Check 2: Rate limiter ---
    let amount = if instruction_data.len() >= 9 {
        read_u64(instruction_data, 1)
    } else {
        0
    };

    let current_slot = if instruction_data.len() >= 17 {
        read_u64(instruction_data, 9)
    } else {
        0
    };

    let max_outflow = read_u64(&data, OFFSET_MAX_OUTFLOW);
    let window_start = read_u64(&data, OFFSET_WINDOW_START);
    let mut current_outflow = read_u64(&data, OFFSET_CURRENT_OUTFLOW);

    // Reset window if expired (60-second windows)
    let window_duration: u64 = 60;
    if current_slot >= window_start + window_duration {
        // New window
        write_u64(&mut data, OFFSET_WINDOW_START, current_slot);
        current_outflow = 0;
    }

    // Check rate limit — use checked arithmetic to prevent overflow
    let new_outflow = current_outflow.checked_add(amount).ok_or(ERR_RATE_LIMIT_EXCEEDED)?;
    if new_outflow > max_outflow {
        msg!("Error: rate limit exceeded");
        return Err(ERR_RATE_LIMIT_EXCEEDED.into());
    }

    // Update outflow
    write_u64(&mut data, OFFSET_CURRENT_OUTFLOW, new_outflow);

    msg!("Transfer allowed through circuit breaker");

    // NOTE: In a production program, actual token transfer CPI would happen here.
    // This skeleton validates the circuit breaker logic only.

    Ok(())
}

/// Update the max outflow threshold.
///
/// Accounts:
///   0 — [signer] authority
///   1 — [writable] config PDA
///
/// Data: [discriminator (1B), new_max_outflow (8B)]
fn process_update_threshold(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let [authority, config] = accounts else {
        return Err(ERR_INVALID_ACCOUNT.into());
    };

    if !authority.is_signer() {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let mut data = config.try_borrow_mut_data()?;

    // Verify authority — full 32-byte comparison
    if !verify_authority(authority, &data) {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let new_max = if instruction_data.len() >= 9 {
        read_u64(instruction_data, 1)
    } else {
        return Err(ERR_INVALID_ACCOUNT.into());
    };

    write_u64(&mut data, OFFSET_MAX_OUTFLOW, new_max);

    msg!("Rate limit threshold updated");

    Ok(())
}
