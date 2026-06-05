use pinocchio::{
    AccountView,
    Address,
    ProgramResult,
    cpi::Signer,
    instruction::cpi::Seed,
};

use pinocchio_token::instructions::{MintTo, Transfer};
use solana_program_log::log;

use crate::state::{PoolState, ERR_PAUSED};

pub const DISC: u8 = 2;

pub fn process_add(
    _program_id: &Address,
    accounts: &mut [AccountView],
    ix_data: &[u8],
) -> ProgramResult {
    let [user, pool_state, user_mint_a, user_mint_b, reserve_a, reserve_b, lp_mint, user_lp, _token_program, _rest @ ..] =
        accounts
    else {
        return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
    };

    if !user.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let pool_data = pool_state.try_borrow()?;

    if pool_data[PoolState::OFFSET_PAUSED] != 0 {
        return Err(ERR_PAUSED.into());
    }

    let amount_a = read_u64(ix_data, 1).unwrap_or(0);
    let amount_b = read_u64(ix_data, 9).unwrap_or(0);

    let reserve_a_data = reserve_a.try_borrow()?;
    let reserve_b_data = reserve_b.try_borrow()?;
    let lp_mint_data = lp_mint.try_borrow()?;

    let reserve_a_amt = read_u64_tok(&reserve_a_data);
    let reserve_b_amt = read_u64_tok(&reserve_b_data);
    let lp_supply = read_u64_lp(&lp_mint_data);

    drop(reserve_a_data);
    drop(reserve_b_data);
    drop(lp_mint_data);

    let lp_to_mint = if lp_supply == 0 {
        // First liquidity: reserves are empty, use input amounts
        let product = (amount_a as u128).saturating_mul(amount_b as u128);
        approx_sqrt(product).saturating_mul(1_000_000)
    } else {
        let ratio_a = (amount_a as u128 * 1_000_000)
            .saturating_div(reserve_a_amt.max(1) as u128);
        let ratio_b = (amount_b as u128 * 1_000_000)
            .saturating_div(reserve_b_amt.max(1) as u128);
        let min_ratio = ratio_a.min(ratio_b);
        ((lp_supply as u128).saturating_mul(min_ratio)).saturating_div(1_000_000) as u64
    };

    let bump = pool_data[PoolState::OFFSET_BUMP];
    let mut mint_a_arr = [0u8; 32];
    mint_a_arr.copy_from_slice(&pool_data[PoolState::OFFSET_MINT_A..PoolState::OFFSET_MINT_A + 32]);
    let mut mint_b_arr = [0u8; 32];
    mint_b_arr.copy_from_slice(&pool_data[PoolState::OFFSET_MINT_B..PoolState::OFFSET_MINT_B + 32]);

    drop(pool_data);

    Transfer::new(&*user_mint_a, &*reserve_a, &*user, amount_a)
        .invoke()?;

    Transfer::new(&*user_mint_b, &*reserve_b, &*user, amount_b)
        .invoke()?;

    let bump_seed = [bump];
    let signer_seeds = [
        Seed::from(b"stable-swap-pool"),
        Seed::from(&mint_a_arr[..]),
        Seed::from(&mint_b_arr[..]),
        Seed::from(&bump_seed[..]),
    ];
    let signer = Signer::from(&signer_seeds);

    MintTo::new(&*lp_mint, &*user_lp, &*pool_state, lp_to_mint)
        .invoke_signed(&[signer])?;

    log("Added liquidity");

    Ok(())
}

fn approx_sqrt(n: u128) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x as u64
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
