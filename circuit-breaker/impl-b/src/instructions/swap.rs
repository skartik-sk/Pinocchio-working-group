use pinocchio::{
    AccountView,
    Address,
    ProgramResult,
    cpi::Signer,
    instruction::cpi::Seed,
    sysvars::clock::Clock,
    sysvars::Sysvar,
};
use pinocchio_system::instructions::CreateAccount;
use pinocchio_token::instructions::Transfer;
use solana_program_log::log;

use crate::state::{PoolState, ERR_CB_TRIGGERED, ERR_INSUFFICIENT, ERR_PAUSED};

pub const INIT_DISC: u8 = 0;
pub const SWAP_DISC: u8 = 1;

pub fn process_init(
    program_id: &Address,
    accounts: &mut [AccountView],
    ix_data: &[u8],
) -> ProgramResult {
    let [authority, mint_a, mint_b, pool_state, _system_program, _token_program, _rest @ ..] =
        accounts
    else {
        return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
    };

    if !authority.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let amp = read_u64(ix_data, 1).unwrap_or(100);

    let (pool_key, bump) = Address::find_program_address(
        &[b"stable-swap-pool", mint_a.address().as_ref(), mint_b.address().as_ref()],
        program_id,
    );

    if pool_state.address() != &pool_key {
        return Err(pinocchio::error::ProgramError::InvalidAccountData);
    }

    CreateAccount {
        from: &*authority,
        to: &*pool_state,
        lamports: 1_000_000,
        space: PoolState::LEN as u64,
        owner: program_id,
    }
    .invoke()?;

    let mut data = pool_state.try_borrow_mut()?;

    data[PoolState::OFFSET_AUTHORITY..PoolState::OFFSET_AUTHORITY + 32]
        .copy_from_slice(authority.address().as_ref());
    data[PoolState::OFFSET_MINT_A..PoolState::OFFSET_MINT_A + 32]
        .copy_from_slice(mint_a.address().as_ref());
    data[PoolState::OFFSET_MINT_B..PoolState::OFFSET_MINT_B + 32]
        .copy_from_slice(mint_b.address().as_ref());
    write_u64(&mut data, PoolState::OFFSET_AMP, amp);
    write_u16(&mut data, PoolState::OFFSET_TRADE_FEE, 25);
    write_u16(&mut data, PoolState::OFFSET_ADMIN_FEE, 5);
    data[PoolState::OFFSET_PAUSED] = 0;
    write_u64(&mut data, PoolState::OFFSET_CB_WINDOW, 60);
    write_u64(&mut data, PoolState::OFFSET_CB_THRESH, 1_000_000_000);
    write_u64(&mut data, PoolState::OFFSET_CB_LAST_VAL, 0);
    write_i64(&mut data, PoolState::OFFSET_CB_LAST_TS, 0);
    data[PoolState::OFFSET_BUMP] = bump;

    log("Pool initialized");

    Ok(())
}

pub fn process_swap(
    _program_id: &Address,
    accounts: &mut [AccountView],
    ix_data: &[u8],
) -> ProgramResult {
    let [user, pool_state, user_source, user_dest, reserve_in, reserve_out, _token_program, _rest @ ..] =
        accounts
    else {
        return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
    };

    if !user.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let pool_data = pool_state.try_borrow_mut()?;

    if pool_data[PoolState::OFFSET_PAUSED] != 0 {
        return Err(ERR_PAUSED.into());
    }

    let amount_in = read_u64(ix_data, 1)?;
    let min_out = read_u64(ix_data, 9).unwrap_or(0);

    let amp = read_u64_from(&pool_data, PoolState::OFFSET_AMP);
    let trade_fee = read_u16(&pool_data, PoolState::OFFSET_TRADE_FEE);

    drop(pool_data);

    let reserve_in_data = reserve_in.try_borrow()?;
    let reserve_out_data = reserve_out.try_borrow()?;

    let reserve_in_amt = read_u64_tok(&reserve_in_data);
    let reserve_out_amt = read_u64_tok(&reserve_out_data);

    drop(reserve_in_data);
    drop(reserve_out_data);

    let amount_out = compute_swap(amount_in, reserve_in_amt, reserve_out_amt, amp, trade_fee)
        .ok_or(ERR_INSUFFICIENT)?;

    if amount_out < min_out {
        return Err(ERR_INSUFFICIENT.into());
    }

    let current_ts = Clock::get()?.unix_timestamp;

    let mut pool_data = pool_state.try_borrow_mut()?;

    if !check_cb(&mut pool_data, amount_in, current_ts)? {
        return Err(ERR_CB_TRIGGERED.into());
    }

    let bump = pool_data[PoolState::OFFSET_BUMP];

    let mut mint_a_arr = [0u8; 32];
    mint_a_arr.copy_from_slice(&pool_data[PoolState::OFFSET_MINT_A..PoolState::OFFSET_MINT_A + 32]);
    let mut mint_b_arr = [0u8; 32];
    mint_b_arr.copy_from_slice(&pool_data[PoolState::OFFSET_MINT_B..PoolState::OFFSET_MINT_B + 32]);

    drop(pool_data);

    Transfer::new(&*user_source, &*reserve_in, &*user, amount_in)
        .invoke()?;

    let bump_seed = [bump];
    let signer_seeds = [
        Seed::from(b"stable-swap-pool"),
        Seed::from(&mint_a_arr[..]),
        Seed::from(&mint_b_arr[..]),
        Seed::from(&bump_seed[..]),
    ];
    let signer = Signer::from(&signer_seeds);

    Transfer::new(&*reserve_out, &*user_dest, &*pool_state, amount_out)
        .invoke_signed(&[signer])?;

    log("Swapped");

    Ok(())
}

fn compute_swap(amount_in: u64, reserve_in: u64, reserve_out: u64, amp: u64, fee_bps: u16) -> Option<u64> {
    let fee = (amount_in as u128)
        .saturating_mul(fee_bps as u128)
        .saturating_div(10000) as u64;
    let amount_with_fee = amount_in.saturating_sub(fee);

    let d = (reserve_in as u128)
        .saturating_mul(reserve_out as u128)
        .saturating_mul(amp as u128 * 4);

    let new_in = (reserve_in as u128).saturating_add(amount_with_fee as u128);

    let new_out = if amp >= 1000 {
        let x = amp as u128 * 4 + d;
        let y = amp as u128 * 4 + 1;
        (x * reserve_out as u128).saturating_div(y)
    } else {
        (reserve_out as u128 * d).saturating_div(new_in)
    };

    Some(reserve_out.saturating_sub(new_out as u64))
}

fn check_cb(data: &mut [u8], amount: u64, ts: i64) -> Result<bool, pinocchio::error::ProgramError> {
    let window = read_u64_from(data, PoolState::OFFSET_CB_WINDOW);
    let threshold = read_u64_from(data, PoolState::OFFSET_CB_THRESH);
    let last_val = read_u64_from(data, PoolState::OFFSET_CB_LAST_VAL);
    let last_ts = read_i64(data, PoolState::OFFSET_CB_LAST_TS);

    let elapsed = ts.saturating_sub(last_ts);
    let decayed = if elapsed as u64 >= window {
        0
    } else {
        (last_val as u128)
            .saturating_mul((window - elapsed as u64) as u128)
            .saturating_div(window as u128) as u64
    };

    let new_agg = decayed.saturating_add(amount);

    if new_agg <= threshold {
        write_u64(data, PoolState::OFFSET_CB_LAST_VAL, new_agg);
        write_i64(data, PoolState::OFFSET_CB_LAST_TS, ts);
        Ok(true)
    } else {
        Ok(false)
    }
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, pinocchio::error::ProgramError> {
    if data.len() < offset + 8 {
        return Err(pinocchio::error::ProgramError::InvalidInstructionData);
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    Ok(u64::from_le_bytes(buf))
}

fn read_u64_from(data: &[u8], offset: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(buf)
}

fn read_u64_tok(data: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[64..72]);
    u64::from_le_bytes(buf)
}

fn read_i64(data: &[u8], offset: usize) -> i64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    i64::from_le_bytes(buf)
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    let mut buf = [0u8; 2];
    buf.copy_from_slice(&data[offset..offset + 2]);
    u16::from_le_bytes(buf)
}

fn write_u64(data: &mut [u8], offset: usize, val: u64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}

fn write_i64(data: &mut [u8], offset: usize, val: i64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}

fn write_u16(data: &mut [u8], offset: usize, val: u16) {
    data[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
}
