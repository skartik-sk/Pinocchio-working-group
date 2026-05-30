use pinocchio::{
    AccountView,
    Address,
    ProgramResult,
    cpi::Signer,
    instruction::cpi::Seed,
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
    let [maker, maker_ata_a, vault, escrow, _token_program, _rest @ ..] = accounts else {
        return Err(pinocchio::error::ProgramError::NotEnoughAccountKeys);
    };

    if !maker.is_signer() {
        return Err(pinocchio::error::ProgramError::MissingRequiredSignature);
    }

    let escrow_data = escrow.try_borrow()?;

    let mut arr = [0u8; 32];
    arr.copy_from_slice(&escrow_data[Escrow::OFFSET_MAKER..Escrow::OFFSET_MAKER + 32]);
    let stored_maker = Address::new_from_array(arr);
    let amount = read_u64_escrow(&escrow_data);
    let bump = escrow_data[Escrow::OFFSET_BUMP];

    drop(escrow_data);

    if maker.address() != &stored_maker {
        return Err(ERR_UNAUTHORIZED.into());
    }

    let bump_seed = [bump];
    let signer_seeds = [
        Seed::from(b"escrow"),
        Seed::from(maker.address().as_ref()),
        Seed::from(&bump_seed[..]),
    ];
    let signer = Signer::from(&signer_seeds);

    Transfer::new(&*vault, &*maker_ata_a, &*escrow, amount)
        .invoke_signed(&[signer.clone()])?;

    CloseAccount::new(&*vault, &*maker, &*escrow)
        .invoke_signed(&[signer])?;

    let escrow_lamports = escrow.lamports();
    maker.set_lamports(maker.lamports() + escrow_lamports);
    escrow.set_lamports(0);

    log("Escrow refunded");

    Ok(())
}
