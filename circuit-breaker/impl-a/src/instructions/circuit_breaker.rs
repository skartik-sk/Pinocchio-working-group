use pinocchio::{
    AccountView,
    Address,
    ProgramResult,
};
use pinocchio_system::instructions::CreateAccount;
use solana_program_log::log;

use crate::{
    state::{CircuitBreaker, ERR_UNAUTHORIZED, ERR_INVALID_ACCOUNT},
};

pub const INIT_DISC: u8 = 3;
pub const UPDATE_DISC: u8 = 4;
pub const PAUSE_DISC: u8 = 5;

pub fn process_init(
    program_id: &Address,
    accounts: &mut [AccountView],
    ix_data: &[u8],
) -> ProgramResult {
    let [authority, cb_pda, system_program, _rest @ ..] = accounts else {
        return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
    };

    if !authority.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let window_sec = read_u64_cb(ix_data, 1).unwrap_or(60);
    let threshold_type = ix_data.get(9).copied().unwrap_or(1);
    let threshold = read_u64_cb(ix_data, 10).unwrap_or(1_000_000);

    let cb_seeds = [b"circuit-breaker", authority.key().as_ref()];
    let (cb_key, bump) = Address::find_program_address(&cb_seeds, program_id);

    if cb_pda.key() != &cb_key {
        return Err(ERR_INVALID_ACCOUNT.into());
    }

    let space = CircuitBreaker::LEN as u64;
    let lamports = 1_000_000;

    CreateAccount {
        from: &authority,
        to: &cb_pda,
        lamports,
        space,
        owner: program_id,
    }
    .invoke()?;

    let mut cb_data = cb_pda.try_borrow_mut()?;

    cb_data[CircuitBreaker::OFFSET_AUTHORITY..CircuitBreaker::OFFSET_AUTHORITY + 32]
        .copy_from_slice(authority.key().as_ref());
    cb_data[CircuitBreaker::OFFSET_PAUSED] = 0;
    write_u64_cb(&mut cb_data, CircuitBreaker::OFFSET_WINDOW_SEC, window_sec);
    cb_data[CircuitBreaker::OFFSET_THRESHOLD_TYPE] = threshold_type;
    write_u64_cb(&mut cb_data, CircuitBreaker::OFFSET_THRESHOLD, threshold);
    write_u64_cb(&mut cb_data, CircuitBreaker::OFFSET_LAST_VALUE, 0);
    write_i64_cb(&mut cb_data, CircuitBreaker::OFFSET_LAST_TS, 0);
    cb_data[CircuitBreaker::OFFSET_BUMP] = bump;

    log("CB initialized");

    Ok(())
}

pub fn process_update(
    _program_id: &Address,
    accounts: &mut [AccountView],
    ix_data: &[u8],
) -> ProgramResult {
    let [authority, cb_pda, _rest @ ..] = accounts else {
        return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
    };

    if !authority.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let cb_data = cb_pda.try_borrow()?;

    if !verify_auth(&authority, &cb_data) {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let mut cb_data = cb_pda.try_borrow_mut()?;

    if ix_data.len() >= 9 {
        write_u64_cb(&mut cb_data, CircuitBreaker::OFFSET_WINDOW_SEC, read_u64_cb(ix_data, 1)?);
    }
    if ix_data.len() >= 10 {
        cb_data[CircuitBreaker::OFFSET_THRESHOLD_TYPE] = ix_data[9];
    }
    if ix_data.len() >= 19 {
        write_u64_cb(&mut cb_data, CircuitBreaker::OFFSET_THRESHOLD, read_u64_cb(ix_data, 10)?);
    }

    log("CB updated");

    Ok(())
}

pub fn process_pause(
    _program_id: &Address,
    accounts: &mut [AccountView],
    ix_data: &[u8],
) -> ProgramResult {
    let [authority, cb_pda, _rest @ ..] = accounts else {
        return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
    };

    if !authority.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let cb_data = cb_pda.try_borrow()?;

    if !verify_auth(&authority, &cb_data) {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let mut cb_data = cb_pda.try_borrow_mut()?;

    let paused = ix_data.get(1).copied().unwrap_or(1);
    cb_data[CircuitBreaker::OFFSET_PAUSED] = paused;

    log("Paused");

    Ok(())
}

pub fn check_and_update_window(
    data: &mut [u8],
    amount: u64,
    current_ts: i64,
    account_balance: u64,
) -> Result<bool, ProgramResult> {
    let window_sec = read_u64_data(data, CircuitBreaker::OFFSET_WINDOW_SEC);
    let threshold_type = data[CircuitBreaker::OFFSET_THRESHOLD_TYPE];
    let threshold = read_u64_data(data, CircuitBreaker::OFFSET_THRESHOLD);
    let last_value = read_u64_data(data, CircuitBreaker::OFFSET_LAST_VALUE);
    let last_ts = read_i64_data(data, CircuitBreaker::OFFSET_LAST_TS);

    let window_elapsed = current_ts.saturating_sub(last_ts);
    let decayed = if window_elapsed as u64 >= window_sec {
        0
    } else {
        (last_value as u128)
            .saturating_mul((window_sec - window_elapsed as u64) as u128)
            .saturating_div(window_sec as u128) as u64
    };

    let new_aggregated = decayed.saturating_add(amount);

    let limit = if threshold_type == 0 {
        (account_balance as u128)
            .saturating_mul(threshold as u128)
            .saturating_div(u64::MAX as u128) as u64
    } else {
        threshold
    };

    if new_aggregated <= limit {
        write_u64_cb(data, CircuitBreaker::OFFSET_LAST_VALUE, new_aggregated);
        write_i64_cb(data, CircuitBreaker::OFFSET_LAST_TS, current_ts);
        Ok(true)
    } else {
        Ok(false)
    }
}

fn verify_auth(account: &AccountView, data: &[u8]) -> bool {
    let stored = &data[CircuitBreaker::OFFSET_AUTHORITY..CircuitBreaker::OFFSET_AUTHORITY + 32];
    let caller = account.key().as_ref();
    let mut diff: u8 = 0;
    for i in 0..32 {
        diff |= stored[i] ^ caller[i];
    }
    diff == 0
}

fn read_u64_cb(data: &[u8], offset: usize) -> Result<u64, ProgramResult> {
    if data.len() < offset + 8 {
        return Err(pinocchio::error::ProgramError::InvalidInstructionData);
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    Ok(u64::from_le_bytes(buf))
}

fn read_u64_data(data: &[u8], offset: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(buf)
}

fn read_i64_data(data: &[u8], offset: usize) -> i64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    i64::from_le_bytes(buf)
}

fn write_u64_cb(data: &mut [u8], offset: usize, val: u64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}

fn write_i64_cb(data: &mut [u8], offset: usize, val: i64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}
