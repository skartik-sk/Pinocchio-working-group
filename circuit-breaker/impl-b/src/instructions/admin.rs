use pinocchio::{
    account_info::AccountInfo,
    pubkey::Pubkey,
    AccountInfo as AccountInfoRef,
    ProgramResult,
};

use pinocchio_token::instructions::Transfer;

use crate::state::{PoolState, ERR_UNAUTHORIZED, ERR_INVALID_ACCOUNT};

pub const PAUSE_DISC: u8 = 3;
pub const UPDATE_CB_DISC: u8 = 4;
pub const EMERGENCY_DISC: u8 = 5;

pub fn process_pause(
    _program_id: &Pubkey,
    accounts: &mut [AccountInfoRef],
    ix_data: &[u8],
) -> ProgramResult {
    let [authority, pool_state, _rest @ ..] = accounts else {
        return Err(pinocchio::ProgramError::NotEnoughAccountKeys);
    };

    if !authority.is_signer() {
        return Err(pinocchio::ProgramError::MissingRequiredSignature);
    }

    let pool_data = pool_state.try_borrow_data()?;

    if !verify_auth(authority, &pool_data) {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let mut pool_data = pool_state.try_borrow_mut_data()?;

    let paused = ix_data.get(1).copied().unwrap_or(1);
    pool_data[PoolState::OFFSET_PAUSED] = paused;

    pinocchio::msg!("Paused: {}", paused);

    Ok(())
}

pub fn process_update_cb(
    _program_id: &Pubkey,
    accounts: &mut [AccountInfoRef],
    ix_data: &[u8],
) -> ProgramResult {
    let [authority, pool_state, _rest @ ..] = accounts else {
        return Err(pinocchio::ProgramError::NotEnoughAccountKeys);
    };

    if !authority.is_signer() {
        return Err(pinocchio::ProgramError::MissingRequiredSignature);
    }

    let pool_data = pool_state.try_borrow_data()?;

    if !verify_auth(authority, &pool_data) {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let mut pool_data = pool_state.try_borrow_mut_data()?;

    if ix_data.len() >= 9 {
        write_u64(&mut pool_data, PoolState::OFFSET_CB_WINDOW, read_u64(ix_data, 1)?);
    }
    if ix_data.len() >= 17 {
        write_u64(&mut pool_data, PoolState::OFFSET_CB_THRESH, read_u64(ix_data, 9)?);
    }

    pinocchio::msg!("CB updated");

    Ok(())
}

pub fn process_emergency(
    program_id: &Pubkey,
    accounts: &mut [AccountInfoRef],
    _ix_data: &[u8],
) -> ProgramResult {
    let [authority, pool_state, reserve_a, reserve_b, dest_a, dest_b, token_program, _rest @ ..] =
        accounts
    else {
        return Err(pinocchio::ProgramError::NotEnoughAccountKeys);
    };

    if !authority.is_signer() {
        return Err(pinocchio::ProgramError::MissingRequiredSignature);
    }

    let pool_data = pool_state.try_borrow_data()?;

    if !verify_auth(authority, &pool_data) {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let reserve_a_data = reserve_a.try_borrow_data()?;
    let reserve_b_data = reserve_b.try_borrow_data()?;

    let amount_a = read_u64_tok(&reserve_a_data);
    let amount_b = read_u64_tok(&reserve_b_data);

    let pool_seeds = [
        b"stable-swap-pool",
        &pool_data[PoolState::OFFSET_MINT_A..PoolState::OFFSET_MINT_A + 32],
        &pool_data[PoolState::OFFSET_MINT_B..PoolState::OFFSET_MINT_B + 32],
        &[pool_data[PoolState::OFFSET_BUMP]],
    ];

    if amount_a > 0 {
        Transfer {
            from: reserve_a,
            to: dest_a,
            authority: pool_state,
            amount: amount_a,
        }
        .invoke_signed(&[&pool_seeds])?;
    }

    if amount_b > 0 {
        Transfer {
            from: reserve_b,
            to: dest_b,
            authority: pool_state,
            amount: amount_b,
        }
        .invoke_signed(&[&pool_seeds])?;
    }

    pinocchio::msg!("Emergency withdraw");

    Ok(())
}

fn verify_auth(account: &AccountInfoRef, data: &[u8]) -> bool {
    let stored = &data[PoolState::OFFSET_AUTHORITY..PoolState::OFFSET_AUTHORITY + 32];
    let caller = account.key().as_ref();
    let mut diff: u8 = 0;
    for i in 0..32 {
        diff |= stored[i] ^ caller[i];
    }
    diff == 0
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, pinocchio::ProgramError> {
    if data.len() < offset + 8 {
        return Err(pinocchio::ProgramError::InvalidInstruction);
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    Ok(u64::from_le_bytes(buf))
}

fn read_u64_tok(data: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[64..72]);
    u64::from_le_bytes(buf)
}

fn write_u64(data: &mut [u8], offset: usize, val: u64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}
