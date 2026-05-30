use pinocchio::{
    AccountView,
    Address,
    ProgramResult,
    cpi::Signer,
    instruction::cpi::Seed,
    sysvars::clock::Clock,
    sysvars::Sysvar,
};

use pinocchio_token::instructions::{CloseAccount, Transfer};
use solana_program_log::log;

use crate::{
    state::{CircuitBreaker, Escrow, ERR_CB_TRIGGERED, ERR_EXPIRED, ERR_INVALID_ACCOUNT, ERR_PAUSED},
};

pub const DISC: u8 = 1;

pub fn process(
    _program_id: &Address,
    accounts: &mut [AccountView],
    _ix_data: &[u8],
) -> ProgramResult {
    let [taker, maker, mint_a, mint_b, maker_ata_a, taker_ata_a, taker_ata_b, vault, escrow, cb_pda, _token_program, _rest @ ..] =
        accounts
    else {
            return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
        };

    if !taker.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let escrow_data = escrow.try_borrow()?;

    let mut arr = [0u8; 32];
    arr.copy_from_slice(&escrow_data[Escrow::OFFSET_MAKER..Escrow::OFFSET_MAKER + 32]);
    let stored_maker = Address::new_from_array(arr);
    arr.copy_from_slice(&escrow_data[Escrow::OFFSET_MINT_A..Escrow::OFFSET_MINT_A + 32]);
    let stored_mint_a = Address::new_from_array(arr);
    arr.copy_from_slice(&escrow_data[Escrow::OFFSET_MINT_B..Escrow::OFFSET_MINT_B + 32]);
    let stored_mint_b = Address::new_from_array(arr);
    let amount = read_u64_escrow(&escrow_data);
    let expiry = read_i64_escrow(&escrow_data);

    drop(escrow_data);

    if maker.address() != &stored_maker {
        return Err(ERR_INVALID_ACCOUNT.into());
    }

    if mint_a.address() != &stored_mint_a {
        return Err(ERR_INVALID_ACCOUNT.into());
    }

    if mint_b.address() != &stored_mint_b {
        return Err(ERR_INVALID_ACCOUNT.into());
    }

    let current_ts = Clock::get()?.unix_timestamp;

    if expiry < current_ts {
        return Err(ERR_EXPIRED.into());
    }

    let cb_data_len = cb_pda.data_len();
    let mut cb_data = cb_pda.try_borrow_mut()?;

    if cb_data_len >= CircuitBreaker::LEN {
        if cb_data[CircuitBreaker::OFFSET_PAUSED] != 0 {
            return Err(ERR_PAUSED.into());
        }

        if !check_and_update_window(&mut cb_data, amount, current_ts, amount)? {
            return Err(ERR_CB_TRIGGERED.into());
        }
    }

    drop(cb_data);

    let escrow_data = escrow.try_borrow()?;
    let bump = escrow_data[Escrow::OFFSET_BUMP];
    drop(escrow_data);

    let bump_seed = [bump];
    let signer_seeds = [
        Seed::from(b"escrow"),
        Seed::from(maker.address().as_ref()),
        Seed::from(&bump_seed[..]),
    ];
    let signer = Signer::from(&signer_seeds);

    Transfer::new(&*vault, &*taker_ata_a, &*escrow, amount)
        .invoke_signed(&[signer.clone()])?;

    Transfer::new(&*taker_ata_b, &*maker_ata_a, &*taker, amount)
        .invoke()?;

    CloseAccount::new(&*vault, &*maker, &*escrow)
        .invoke_signed(&[signer])?;

    let escrow_lamports = escrow.lamports();
    maker.set_lamports(maker.lamports() + escrow_lamports);
    escrow.set_lamports(0);

    log("Escrow taken");

    Ok(())
}

pub fn read_u64_escrow(data: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[Escrow::OFFSET_AMOUNT..Escrow::OFFSET_AMOUNT + 8]);
    u64::from_le_bytes(buf)
}

fn read_i64_escrow(data: &[u8]) -> i64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[Escrow::OFFSET_EXPIRY..Escrow::OFFSET_EXPIRY + 8]);
    i64::from_le_bytes(buf)
}

fn check_and_update_window(
    data: &mut [u8],
    amount: u64,
    current_ts: i64,
    account_balance: u64,
) -> Result<bool, pinocchio::error::ProgramError> {
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
        write_u64_data(data, CircuitBreaker::OFFSET_LAST_VALUE, new_aggregated);
        write_i64_data(data, CircuitBreaker::OFFSET_LAST_TS, current_ts);
        Ok(true)
    } else {
        Ok(false)
    }
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

fn write_u64_data(data: &mut [u8], offset: usize, val: u64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}

fn write_i64_data(data: &mut [u8], offset: usize, val: i64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}
