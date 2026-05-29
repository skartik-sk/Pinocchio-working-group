use pinocchio::{
    account_info::AccountInfo,
    pubkey::Pubkey,
    AccountInfo as AccountInfoRef,
    ProgramResult,
};

use pinocchio_token::instructions::{MintTo, Transfer};

use crate::state::{PoolState, ERR_PAUSED};

pub const DISC: u8 = 2;

pub fn process_add(
    program_id: &Pubkey,
    accounts: &mut [AccountInfoRef],
    ix_data: &[u8],
) -> ProgramResult {
    let [user, pool_state, user_mint_a, user_mint_b, reserve_a, reserve_b, lp_mint, user_lp, token_program, _rest @ ..] =
        accounts
    else {
        return Err(pinocchio::ProgramError::NotEnoughAccountKeys);
    };

    if !user.is_signer() {
        return Err(pinocchio::ProgramError::MissingRequiredSignature);
    }

    let pool_data = pool_state.try_borrow_data()?;

    if pool_data[PoolState::OFFSET_PAUSED] != 0 {
        return Err(ERR_PAUSED.into());
    }

    let amount_a = read_u64(ix_data, 1).unwrap_or(0);
    let amount_b = read_u64(ix_data, 9).unwrap_or(0);

    let reserve_a_data = reserve_a.try_borrow_data()?;
    let reserve_b_data = reserve_b.try_borrow_data()?;
    let lp_mint_data = lp_mint.try_borrow_data()?;

    let reserve_a_amt = read_u64_tok(&reserve_a_data);
    let reserve_b_amt = read_u64_tok(&reserve_b_data);
    let lp_supply = read_u64_lp(&lp_mint_data);

    let lp_to_mint = if lp_supply == 0 {
        ((reserve_a_amt as u128) * (reserve_b_amt as u128))
            .saturating_mul(1_000_000)
            .sqrt() as u64
    } else {
        let ratio_a = (amount_a as u128 * 1_000_000)
            .saturating_div(reserve_a_amt as u128);
        let ratio_b = (amount_b as u128 * 1_000_000)
            .saturating_div(reserve_b_amt as u128);
        let min_ratio = ratio_a.min(ratio_b);
        ((lp_supply as u128) * min_ratio).saturating_div(1_000_000) as u64
    };

    let pool_seeds = [
        b"stable-swap-pool",
        &pool_data[PoolState::OFFSET_MINT_A..PoolState::OFFSET_MINT_A + 32],
        &pool_data[PoolState::OFFSET_MINT_B..PoolState::OFFSET_MINT_B + 32],
        &[pool_data[PoolState::OFFSET_BUMP]],
    ];

    Transfer {
        from: user_mint_a,
        to: reserve_a,
        authority: user,
        amount: amount_a,
    }
    .invoke()?;

    Transfer {
        from: user_mint_b,
        to: reserve_b,
        authority: user,
        amount: amount_b,
    }
    .invoke()?;

    MintTo {
        mint: lp_mint,
        to: user_lp,
        authority: pool_state,
        amount: lp_to_mint,
    }
    .invoke_signed(&[&pool_seeds])?;

    pinocchio::msg!("Added liquidity");

    Ok(())
}

fn read_u64(data: &[u8], offset: usize) -> Option<u64> {
    if data.len() < offset + 8 {
        return None;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    Some(u64::from_le_bytes(buf))
}

fn read_u64_tok(data: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[64..72]);
    u64::from_le_bytes(buf)
}

fn read_u64_lp(data: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[36..44]);
    u64::from_le_bytes(buf)
}
