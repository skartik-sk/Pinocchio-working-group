use pinocchio::{
    AccountView,
    Address,
    ProgramResult,
};
use pinocchio_system::instructions::CreateAccount;
use pinocchio_token::instructions::Transfer;
use solana_program_log::log;

use crate::{
    state::{Escrow, ERR_PAUSED, ERR_INVALID_ACCOUNT},
};

pub const DISC: u8 = 0;

pub fn process(
    program_id: &Address,
    accounts: &mut [AccountView],
    ix_data: &[u8],
) -> ProgramResult {
    let [maker, mint_a, mint_b, maker_ata_a, vault, escrow, cb_pda, system_program, token_program, _rest @ ..] =
        accounts
    else {
            return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
        };

    if !maker.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let amount = read_u64(ix_data, 1)?;
    let expiry = read_i64(ix_data, 9).unwrap_or(i64::MAX);

    let escrow_seeds = [b"escrow", maker.key().as_ref()];
    let (escrow_key, bump) = Address::find_program_address(&escrow_seeds, program_id);

    if escrow.key() != &escrow_key {
        return Err(ERR_INVALID_ACCOUNT.into());
    }

    let cb_seeds = [b"circuit-breaker", maker.key().as_ref()];
    let (cb_key, _cb_bump) = Address::find_program_address(&cb_seeds, program_id);

    if cb_pda.key() != &cb_key {
        return Err(ERR_INVALID_ACCOUNT.into());
    }

    let mut cb_data = cb_pda.try_borrow_mut()?;

    if cb_pda.data_len() >= crate::state::CircuitBreaker::LEN {
        if cb_data[crate::state::CircuitBreaker::OFFSET_PAUSED] != 0 {
            return Err(ERR_PAUSED.into());
        }
    }

    let space = Escrow::LEN as u64;
    let lamports = 1_000_000;

    CreateAccount {
        from: &maker,
        to: &escrow,
        lamports,
        space,
        owner: program_id,
    }
    .invoke()?;

    let mut escrow_data = escrow.try_borrow_mut()?;

    escrow_data[Escrow::OFFSET_MAKER..Escrow::OFFSET_MAKER + 32]
        .copy_from_slice(maker.key().as_ref());
    escrow_data[Escrow::OFFSET_MINT_A..Escrow::OFFSET_MINT_A + 32]
        .copy_from_slice(mint_a.key().as_ref());
    escrow_data[Escrow::OFFSET_MINT_B..Escrow::OFFSET_MINT_B + 32]
        .copy_from_slice(mint_b.key().as_ref());
    write_u64(&mut escrow_data, Escrow::OFFSET_AMOUNT, amount);
    write_i64(&mut escrow_data, Escrow::OFFSET_EXPIRY, expiry);
    escrow_data[Escrow::OFFSET_BUMP] = bump;

    drop(escrow_data);

    Transfer {
        from: &maker_ata_a,
        to: &vault,
        authority: &maker,
        amount,
    }
    .invoke_signed(&[&escrow_seeds, &[&[bump]]])?;

    log("Escrow created");

    Ok(())
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, pinocchio::error::ProgramError> {
    let mut buf = [0u8; 8];
    let end = offset + 8;
    if data.len() < end {
        return Err(pinocchio::error::ProgramError::InvalidInstructionData);
    }
    buf.copy_from_slice(&data[offset..end]);
    Ok(u64::from_le_bytes(buf))
}

fn read_i64(data: &[u8], offset: usize) -> Option<i64> {
    let mut buf = [0u8; 8];
    let end = offset + 8;
    if data.len() < end {
        return None;
    }
    buf.copy_from_slice(&data[offset..end]);
    Some(i64::from_le_bytes(buf))
}

fn write_u64(data: &mut [u8], offset: usize, val: u64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}

fn write_i64(data: &mut [u8], offset: usize, val: i64) {
    data[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}
