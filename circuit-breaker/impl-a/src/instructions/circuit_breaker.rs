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
    let [authority, cb_pda, _system_program, _rest @ ..] = accounts else {
        return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
    };

    if !authority.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let window_sec = read_u64_cb(ix_data, 1).unwrap_or(60);
    let threshold_type = ix_data.get(9).copied().unwrap_or(1);
    let threshold = read_u64_cb(ix_data, 10).unwrap_or(1_000_000);

    let (cb_key, bump) = Address::find_program_address(
        &[b"circuit-breaker", authority.address().as_ref()],
        program_id,
    );

    if cb_pda.address() != &cb_key {
        return Err(ERR_INVALID_ACCOUNT.into());
    }

    let space = CircuitBreaker::LEN as u64;
    let lamports = 1_000_000;

    CreateAccount {
        from: &*authority,
        to: &*cb_pda,
        lamports,
        space,
        owner: program_id,
    }
    .invoke()?;

    let mut cb_data = cb_pda.try_borrow_mut()?;

    cb_data[CircuitBreaker::OFFSET_AUTHORITY..CircuitBreaker::OFFSET_AUTHORITY + 32]
        .copy_from_slice(authority.address().as_ref());
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

    drop(cb_data);

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

    drop(cb_data);

    let mut cb_data = cb_pda.try_borrow_mut()?;

    let paused = ix_data.get(1).copied().unwrap_or(1);
    cb_data[CircuitBreaker::OFFSET_PAUSED] = paused;

    log("Paused");

    Ok(())
}

fn verify_auth(account: &AccountView, data: &[u8]) -> bool {
    let stored = &data[CircuitBreaker::OFFSET_AUTHORITY..CircuitBreaker::OFFSET_AUTHORITY + 32];
    let caller = account.address().as_ref();
    let mut diff: u8 = 0;
    for i in 0..32 {
        diff |= stored[i] ^ caller[i];
    }
    diff == 0
}

fn read_u64_cb(data: &[u8], offset: usize) -> Result<u64, pinocchio::error::ProgramError> {
    if data.len() < offset + 8 {
        return Err(pinocchio::error::ProgramError::InvalidInstructionData);
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    Ok(u64::from_le_bytes(buf))
}

fn write_u64_cb(data: &mut [u8], offset: usize, val: u64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}

fn write_i64_cb(data: &mut [u8], offset: usize, val: i64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}
