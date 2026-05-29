use pinocchio::{
    AccountView,
    Address,
    ProgramResult,
};

use pinocchio_token::instructions::{CloseAccount, Transfer};
use solana_program_log::log;

use crate::{state::{Escrow, ERR_UNAUTHORIZED}, instructions::take::read_u64_escrow};

pub const DISC: u8 = 2;

pub fn process(
    _program_id: &Address,
    accounts: &mut [AccountView],
    _ix_data: &[u8],
) -> ProgramResult {
    let [maker, maker_ata_a, vault, escrow, token_program, _rest @ ..] = accounts else {
        return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
    };

    if !maker.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let escrow_data = escrow.try_borrow()?;

    let stored_maker = Address::from_bytes(&escrow_data[Escrow::OFFSET_MAKER..Escrow::OFFSET_MAKER + 32]);
    let amount = read_u64_escrow(&escrow_data);
    let bump = escrow_data[Escrow::OFFSET_BUMP];

    drop(escrow_data);

    if &stored_maker != maker.key() {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let escrow_seeds = [b"escrow", maker.key().as_ref()];

    Transfer {
        from: &vault,
        to: &maker_ata_a,
        authority: &escrow,
        amount,
    }
    .invoke_signed(&[&escrow_seeds, &[&[bump]]])?;

    CloseAccount {
        account: &vault,
        destination: &maker,
        authority: &escrow,
    }
    .invoke_signed(&[&escrow_seeds, &[&[bump]]])?;

    unsafe {
        *maker.try_borrow_mut_lamports_unchecked()? += *escrow.try_borrow_lamports_unchecked()?;
        *escrow.try_borrow_mut_lamports_unchecked()? = 0;
    }

    log("Escrow refunded");

    Ok(())
}
