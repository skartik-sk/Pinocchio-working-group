//! Circuit Breaker - Implementation B
//!
//! Implements:
//!   1. Oracle Guardrails (Price-Based circuit breaker)
//!      - Monitors oracle price deviation from moving average
//!      - Trips when confidence interval is too wide
//!   2. SPL Token-2022 Transfer Hook integration
//!      - Enforces circuit breaker logic at the token transfer level

use pinocchio::{
    AccountView, Address, entrypoint, msg, ProgramResult,
};

// ---------- Instruction discriminators ----------

const INITIALIZE: u8 = 0;
const UPDATE_PRICE: u8 = 1;
const TRANSFER_HOOK: u8 = 2;
const SET_PARAMS: u8 = 3;
const FORCE_PAUSE: u8 = 4;

// ---------- Error codes ----------

const ERR_ORACLE_DEVIATION: u32 = 7000;
const ERR_CONFIDENCE_TOO_WIDE: u32 = 7001;
const ERR_PAUSED: u32 = 7002;
const ERR_UNAUTHORIZED: u32 = 7003;
const ERR_INVALID_ACCOUNT: u32 = 7004;
const ERR_PRICE_STALE: u32 = 7005;

// ---------- State layout (oracle config PDA) ----------
// Byte 0:       paused (bool)
// Byte 1..33:   authority (full 32-byte admin pubkey)
// Byte 33..65:  oracle_updater (full 32-byte oracle pusher pubkey)
// Byte 65..73:  current_price (u64 — scaled by 1e6)
// Byte 73..81:  moving_avg (u64 — scaled by 1e6)
// Byte 81..89:  confidence_interval (u64 — scaled by 1e6)
// Byte 89..97:  max_deviation_bps (u64 — basis points, e.g. 500 = 5%)
// Byte 97..105: max_confidence (u64 — max acceptable confidence)
// Byte 105..113: last_update_slot (u64)
// Byte 113..121: stale_threshold_slots (u64)

const OFFSET_PAUSED: usize = 0;
const OFFSET_AUTHORITY: usize = 1;
const AUTHORITY_LEN: usize = 32;
const OFFSET_ORACLE_UPDATER: usize = OFFSET_AUTHORITY + AUTHORITY_LEN; // 33
const ORACLE_UPDATER_LEN: usize = 32;
const OFFSET_CURRENT_PRICE: usize = OFFSET_ORACLE_UPDATER + ORACLE_UPDATER_LEN; // 65
const OFFSET_MOVING_AVG: usize = OFFSET_CURRENT_PRICE + 8; // 73
const OFFSET_CONFIDENCE: usize = OFFSET_MOVING_AVG + 8; // 81
const OFFSET_MAX_DEVIATION: usize = OFFSET_CONFIDENCE + 8; // 89
const OFFSET_MAX_CONFIDENCE: usize = OFFSET_MAX_DEVIATION + 8; // 97
const OFFSET_LAST_UPDATE: usize = OFFSET_MAX_CONFIDENCE + 8; // 105
const OFFSET_STALE_THRESHOLD: usize = OFFSET_LAST_UPDATE + 8; // 113

// ---------- Helpers ----------

fn read_u64(data: &[u8], offset: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(buf)
}

fn write_u64(data: &mut [u8], offset: usize, val: u64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}

/// Compute absolute difference between two u64 values.
fn abs_diff(a: u64, b: u64) -> u64 {
    if a > b { a - b } else { b - a }
}

/// Constant-time comparison of 32-byte pubkeys to prevent timing attacks.
fn verify_pubkey(account: &AccountView, data: &[u8], offset: usize) -> bool {
    let stored = &data[offset..offset + 32];
    let caller = account.key().as_ref();
    let mut diff: u8 = 0;
    for i in 0..32 {
        diff |= stored[i] ^ caller[i];
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
        UPDATE_PRICE => process_update_price(program_id, accounts, instruction_data),
        TRANSFER_HOOK => process_transfer_hook(program_id, accounts, instruction_data),
        SET_PARAMS => process_set_params(program_id, accounts, instruction_data),
        FORCE_PAUSE => process_force_pause(program_id, accounts, instruction_data),
        _ => {
            msg!("Error: invalid instruction discriminator");
            Err(ERR_INVALID_ACCOUNT.into())
        }
    }
}

/// Initialize the oracle guardrail config PDA.
///
/// Accounts:
///   0 — [signer, writable] authority (admin)
///   1 — [writable]         config PDA
///
/// Data: [discriminator (1B), max_deviation_bps (8B), max_confidence (8B), stale_threshold (8B)]
///       + oracle_updater pubkey (32B) appended after params
fn process_initialize(
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

    // Defaults
    let max_deviation = if instruction_data.len() >= 9 {
        read_u64(instruction_data, 1)
    } else {
        500 // 5% default
    };

    let max_confidence = if instruction_data.len() >= 17 {
        read_u64(instruction_data, 9)
    } else {
        100_000 // 0.1 in 1e6 scale
    };

    let stale_threshold = if instruction_data.len() >= 25 {
        read_u64(instruction_data, 17)
    } else {
        150 // ~60 seconds at ~400ms/slot
    };

    // Write initial state — store FULL 32-byte admin pubkey
    data[OFFSET_PAUSED] = 0;
    let auth_key = authority.key();
    data[OFFSET_AUTHORITY..OFFSET_AUTHORITY + AUTHORITY_LEN]
        .copy_from_slice(auth_key.as_ref());

    // Store oracle updater — either from instruction data or default to authority
    if instruction_data.len() >= 57 {
        // 25 (params) + 32 (pubkey)
        data[OFFSET_ORACLE_UPDATER..OFFSET_ORACLE_UPDATER + ORACLE_UPDATER_LEN]
            .copy_from_slice(&instruction_data[25..57]);
    } else {
        // Default: oracle updater = authority
        data[OFFSET_ORACLE_UPDATER..OFFSET_ORACLE_UPDATER + ORACLE_UPDATER_LEN]
            .copy_from_slice(auth_key.as_ref());
    }

    write_u64(&mut data, OFFSET_CURRENT_PRICE, 0);
    write_u64(&mut data, OFFSET_MOVING_AVG, 0);
    write_u64(&mut data, OFFSET_CONFIDENCE, 0);
    write_u64(&mut data, OFFSET_MAX_DEVIATION, max_deviation);
    write_u64(&mut data, OFFSET_MAX_CONFIDENCE, max_confidence);
    write_u64(&mut data, OFFSET_LAST_UPDATE, 0);
    write_u64(&mut data, OFFSET_STALE_THRESHOLD, stale_threshold);

    msg!("Oracle guardrail circuit breaker initialized");

    Ok(())
}

/// Update oracle price feed (called by authorized oracle pusher).
///
/// Accounts:
///   0 — [signer] oracle updater (must match stored oracle_updater pubkey)
///   1 — [writable] config PDA
///
/// Data: [discriminator (1B), price (8B), confidence (8B), current_slot (8B)]
fn process_update_price(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let [updater, config] = accounts else {
        return Err(ERR_INVALID_ACCOUNT.into());
    };

    // Authorization: updater must sign and match stored oracle_updater
    if !updater.is_signer() {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let mut data = config.try_borrow_mut_data()?;

    if !verify_pubkey(updater, &data, OFFSET_ORACLE_UPDATER) {
        msg!("Error: unauthorized oracle updater");
        return Err(ERR_UNAUTHORIZED.into());
    }

    let price = if instruction_data.len() >= 9 {
        read_u64(instruction_data, 1)
    } else {
        return Err(ERR_INVALID_ACCOUNT.into());
    };

    let confidence = if instruction_data.len() >= 17 {
        read_u64(instruction_data, 9)
    } else {
        0
    };

    let current_slot = if instruction_data.len() >= 25 {
        read_u64(instruction_data, 17)
    } else {
        0
    };

    // Update moving average using 128-bit arithmetic to preserve precision
    // EMA: new_avg = (old_avg * 4 + price) / 5
    let old_avg = read_u64(&data, OFFSET_MOVING_AVG);
    let new_avg = if old_avg == 0 {
        price // first update
    } else {
        let wide: u128 = (old_avg as u128) * 4 + (price as u128);
        (wide / 5) as u64
    };

    write_u64(&mut data, OFFSET_CURRENT_PRICE, price);
    write_u64(&mut data, OFFSET_MOVING_AVG, new_avg);
    write_u64(&mut data, OFFSET_CONFIDENCE, confidence);
    write_u64(&mut data, OFFSET_LAST_UPDATE, current_slot);

    msg!("Oracle price updated");

    Ok(())
}

/// SPL Token-2022 Transfer Hook — enforces circuit breaker at token level.
///
/// This is the cutting-edge approach: the blockchain itself rejects transfers
/// when the circuit breaker is tripped.
///
/// Accounts (per Token-2022 Transfer Hook standard):
///   0 — source account
///   1 — destination account
///   2 — [writable] config PDA (extra accounts metas)
///   3 — authority (signer)
///   4 — mint
///
/// Data: [discriminator (1B), amount (8B), current_slot (8B)]
fn process_transfer_hook(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    // We need at least the config PDA to check state
    if accounts.len() < 3 {
        return Err(ERR_INVALID_ACCOUNT.into());
    }

    let config = &accounts[2];
    let data = config.try_borrow_mut_data()?;

    // --- Guardrail 1: Is the breaker manually paused? ---
    if data[OFFSET_PAUSED] == 1 {
        msg!("Transfer rejected: circuit breaker is PAUSED");
        return Err(ERR_PAUSED.into());
    }

    let current_slot = if instruction_data.len() >= 17 {
        read_u64(instruction_data, 9)
    } else {
        0
    };

    // --- Guardrail 2: Is the price data stale? ---
    let last_update = read_u64(&data, OFFSET_LAST_UPDATE);
    let stale_threshold = read_u64(&data, OFFSET_STALE_THRESHOLD);
    if last_update > 0 && current_slot > last_update + stale_threshold {
        msg!("Transfer rejected: oracle price is STALE");
        return Err(ERR_PRICE_STALE.into());
    }

    // --- Guardrail 3: Price deviation from moving average ---
    let current_price = read_u64(&data, OFFSET_CURRENT_PRICE);
    let moving_avg = read_u64(&data, OFFSET_MOVING_AVG);

    if moving_avg > 0 {
        let deviation = abs_diff(current_price, moving_avg);
        // deviation_bps = (deviation * 10000) / moving_avg
        let deviation_bps = (deviation * 10_000) / moving_avg;
        let max_deviation_bps = read_u64(&data, OFFSET_MAX_DEVIATION);

        if deviation_bps > max_deviation_bps {
            msg!("Transfer rejected: price deviation exceeds threshold");
            return Err(ERR_ORACLE_DEVIATION.into());
        }
    }

    // --- Guardrail 4: Oracle confidence interval too wide ---
    let confidence = read_u64(&data, OFFSET_CONFIDENCE);
    let max_confidence = read_u64(&data, OFFSET_MAX_CONFIDENCE);
    if confidence > max_confidence {
        msg!("Transfer rejected: oracle confidence interval too wide");
        return Err(ERR_CONFIDENCE_TOO_WIDE.into());
    }

    msg!("Transfer hook: circuit breaker checks PASSED — transfer allowed");

    // If we reach here, the transfer is allowed.
    // Token-2022 will proceed with the token transfer.

    Ok(())
}

/// Update oracle guardrail parameters.
///
/// Accounts:
///   0 — [signer] authority (admin)
///   1 — [writable] config PDA
///
/// Data: [discriminator (1B), max_deviation_bps (8B), max_confidence (8B), stale_threshold (8B)]
fn process_set_params(
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

    // Verify admin authority — full 32-byte comparison
    if !verify_pubkey(authority, &data, OFFSET_AUTHORITY) {
        return Err(ERR_UNAUTHORIZED.into());
    }

    if instruction_data.len() >= 9 {
        write_u64(&mut data, OFFSET_MAX_DEVIATION, read_u64(instruction_data, 1));
    }
    if instruction_data.len() >= 17 {
        write_u64(&mut data, OFFSET_MAX_CONFIDENCE, read_u64(instruction_data, 9));
    }
    if instruction_data.len() >= 25 {
        write_u64(&mut data, OFFSET_STALE_THRESHOLD, read_u64(instruction_data, 17));
    }

    msg!("Oracle guardrail parameters updated");

    Ok(())
}

/// Emergency force-pause (admin override).
///
/// Accounts:
///   0 — [signer] authority (admin)
///   1 — [writable] config PDA
///
/// Data: [discriminator (1B), paused (1B: 0 or 1)]
fn process_force_pause(
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

    // Verify admin authority — full 32-byte comparison
    if !verify_pubkey(authority, &data, OFFSET_AUTHORITY) {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let paused = instruction_data.get(1).copied().unwrap_or(1);
    data[OFFSET_PAUSED] = paused;

    if paused == 1 {
        msg!("EMERGENCY: Circuit breaker FORCE-PAUSED");
    } else {
        msg!("Circuit breaker RESUMED");
    }

    Ok(())
}
