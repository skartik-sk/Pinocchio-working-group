use pinocchio::{
    AccountView,
    Address,
    ProgramResult,
    cpi::Signer,
    instruction::cpi::Seed,
};

use pinocchio_token::instructions::Transfer;
use solana_program_log::log;

use crate::state::{PoolState, ERR_UNAUTHORIZED};

pub const PAUSE_DISC: u8 = 3;
pub const UPDATE_CB_DISC: u8 = 4;
pub const EMERGENCY_DISC: u8 = 5;

pub fn process_pause(
    _program_id: &Address,
    accounts: &mut [AccountView],
    ix_data: &[u8],
) -> ProgramResult {
    let [authority, pool_state, _rest @ ..] = accounts else {
        return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
    };

    if !authority.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let pool_data = pool_state.try_borrow()?;

    if !verify_auth(&authority, &pool_data) {
        return Err(ERR_UNAUTHORIZED.into());
    }

    drop(pool_data);

    let mut pool_data = pool_state.try_borrow_mut()?;

    let paused = ix_data.get(1).copied().unwrap_or(1);
    pool_data[PoolState::OFFSET_PAUSED] = paused;

    log("Paused");

    Ok(())
}

pub fn process_update_cb(
    _program_id: &Address,
    accounts: &mut [AccountView],
    ix_data: &[u8],
) -> ProgramResult {
    let [authority, pool_state, _rest @ ..] = accounts else {
        return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
    };

    if !authority.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let pool_data = pool_state.try_borrow()?;

    if !verify_auth(&authority, &pool_data) {
        return Err(ERR_UNAUTHORIZED.into());
    }

    drop(pool_data);

    let mut pool_data = pool_state.try_borrow_mut()?;

    if ix_data.len() >= 9 {
        write_u64(&mut pool_data, PoolState::OFFSET_CB_WINDOW, read_u64(ix_data, 1)?);
    }
    if ix_data.len() >= 17 {
        write_u64(&mut pool_data, PoolState::OFFSET_CB_THRESH, read_u64(ix_data, 9)?);
    }

    log("CB updated");

    Ok(())
}

pub fn process_emergency(
    _program_id: &Address,
    accounts: &mut [AccountView],
    _ix_data: &[u8],
) -> ProgramResult {
    let [authority, pool_state, reserve_a, reserve_b, dest_a, dest_b, _token_program, _rest @ ..] =
        accounts
    else {
        return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
    };

    if !authority.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let pool_data = pool_state.try_borrow()?;

    if !verify_auth(&authority, &pool_data) {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let reserve_a_data = reserve_a.try_borrow()?;
    let reserve_b_data = reserve_b.try_borrow()?;

    let amount_a = read_u64_tok(&reserve_a_data);
    let amount_b = read_u64_tok(&reserve_b_data);

    drop(reserve_a_data);
    drop(reserve_b_data);

    let bump = pool_data[PoolState::OFFSET_BUMP];
    let mut mint_a_arr = [0u8; 32];
    mint_a_arr.copy_from_slice(&pool_data[PoolState::OFFSET_MINT_A..PoolState::OFFSET_MINT_A + 32]);
    let mut mint_b_arr = [0u8; 32];
    mint_b_arr.copy_from_slice(&pool_data[PoolState::OFFSET_MINT_B..PoolState::OFFSET_MINT_B + 32]);

    drop(pool_data);

    let bump_seed = [bump];
    let signer_seeds = [
        Seed::from(b"stable-swap-pool"),
        Seed::from(&mint_a_arr[..]),
        Seed::from(&mint_b_arr[..]),
        Seed::from(&bump_seed[..]),
    ];
    let signer = Signer::from(&signer_seeds);

    if amount_a > 0 {
        Transfer::new(&*reserve_a, &*dest_a, &*pool_state, amount_a)
            .invoke_signed(&[signer.clone()])?;
    }

    if amount_b > 0 {
        Transfer::new(&*reserve_b, &*dest_b, &*pool_state, amount_b)
            .invoke_signed(&[signer])?;
    }

    log("Emergency withdraw");

    Ok(())
}

fn verify_auth(account: &AccountView, data: &[u8]) -> bool {
    let stored = &data[PoolState::OFFSET_AUTHORITY..PoolState::OFFSET_AUTHORITY + 32];
    let caller = account.address().as_ref();
    let mut diff: u8 = 0;
    for i in 0..32 {
        diff |= stored[i] ^ caller[i];
    }
    diff == 0
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, pinocchio::error::ProgramError> {
    if data.len() < offset + 8 {
        return Err(pinocchio::error::ProgramError::InvalidInstructionData);
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
