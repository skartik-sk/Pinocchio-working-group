use pinocchio::{
    AccountView, Address, ProgramResult, cpi::Signer, entrypoint, error::ProgramError,
    instruction::seeds,
};

use pinocchio_token_2022::instructions::{
    CloseAccount,  TransferChecked
};
use pinocchio_token_2022::state::{
    Account, Mint, StateWithExtensions,
};



entrypoint!(process_instruction);

pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    _instruction_data: &[u8],
) -> ProgramResult {
    let [
        nested_ata,
        nested_token_mint,
        destination_ata,
        owner_ata,
        owner_token_mint,
        wallet,
        owner_token_program,
        remaining @ ..,
    ] = accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // Optional to specify nested token program if different from owner one
    let nested_token_program = remaining.first().unwrap_or(owner_token_program);

    // `owner_ata` must be the canonical ATA for wallet & `owner_token_mint`
    // Seed order: [wallet, mint, token_program]
    let Some((derived_owner_ata, bump_seed)) = Address::derive_program_address(
        &[
            &wallet.address().to_bytes(),
            &owner_token_mint.address().to_bytes(),
            &owner_token_program.address().to_bytes(),
        ],
        program_id,
    ) else {
        return Err(ProgramError::InvalidAccountData);
    };
    if derived_owner_ata != *owner_ata.address() {
        return Err(ProgramError::InvalidSeeds);
    }

    // `nested_ata` must be derived from owner_ata as the "wallet".
    // The `owner_ata` address was mistakenly used where a wallet address should have been.
    // Seed order: [owner_ata, nested_mint, nested_token_program]
    let Some((derived_nested_ata, _nested_bump_seed)) = Address::derive_program_address(
        &[
            &owner_ata.address().to_bytes(),
            &nested_token_mint.address().to_bytes(),
            &nested_token_program.address().to_bytes(),
        ],
        program_id,
    ) else {
        return Err(ProgramError::InvalidAccountData);
    };
    if derived_nested_ata != *nested_ata.address() {
        return Err(ProgramError::InvalidSeeds);
    }

    // `destination_ata` must be the wallet's correct ATA for the nested mint
    // Seed order: [wallet, nested_mint, nested_token_program]
    let Some((derived_destination_ata, _destination_bump_seed)) = Address::derive_program_address(
        &[
            &wallet.address().to_bytes(),
            &nested_token_mint.address().to_bytes(),
            &nested_token_program.address().to_bytes(),
        ],
        program_id,
    ) else {
        return Err(ProgramError::InvalidAccountData);
    };
    if derived_destination_ata != *destination_ata.address() {
        return Err(ProgramError::InvalidSeeds);
    }

    // Only the wallet holder can trigger recovery
    if !wallet.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // The owner mint must belong to the token program we will CPI into
    if !owner_token_mint.owned_by(owner_token_program.address()) {
        return Err(ProgramError::IllegalOwner);
    }

    // The owner ATA must also belong to that token program so it can sign as
    // the nested account authority during the recovery CPIs
    if !owner_ata.owned_by(owner_token_program.address()) {
        return Err(ProgramError::IllegalOwner);
    }

    let owner_account_data = owner_ata.try_borrow()?;
    let owner_account = StateWithExtensions::<Account>::from_bytes(&owner_account_data)?;

    // The wallet must actually control this ATA
    if owner_account.base.owner() != wallet.address() {
        return Err(ProgramError::IllegalOwner);
    }
    drop(owner_account_data);

    // The nested ATA must belong to the same token program so its balance can be transferred
    if !nested_ata.owned_by(nested_token_program.address()) {
        return Err(ProgramError::IllegalOwner);
    }

    let nested_account_data = nested_ata.try_borrow()?;
    let nested_account = StateWithExtensions::<Account>::from_bytes(&nested_account_data)?;

    // Confirming this is genuinely a nested ATA, not an arbitrary token account
    if nested_account.base.owner() != owner_ata.address() {
        return Err(ProgramError::IllegalOwner);
    }

    // The nested mint must match the token program
    if !nested_token_mint.owned_by(nested_token_program.address()) {
        return Err(ProgramError::IllegalOwner);
    }

    let nested_mint_data = nested_token_mint.try_borrow()?;
    let nested_mint = StateWithExtensions::<Mint>::from_bytes(&nested_mint_data)?;
    let amount = nested_account.base.amount();
    let decimals = nested_mint.base.decimals();
    drop(nested_account_data);

    // Signing seeds must match the derivation order: [wallet, mint, token_program, bump]
    let bump_ref = &[bump_seed];
    let seeds = seeds!(
        wallet.address().as_ref(),
        owner_token_mint.address().as_ref(),
        owner_token_program.address().as_ref(),
        bump_ref
    );

    // Move all tokens from the nested ATA to the wallet's correct ATA
    TransferChecked {
        from: nested_ata,
        mint: nested_token_mint,
        to: destination_ata,
        authority: owner_ata,
        amount,
        decimals,
        token_program: nested_token_program.address(),
    }
    .invoke_signed(&[Signer::from(&seeds)])?;

    // Close the now-empty nested ATA and return its rent lamports to the wallet
    CloseAccount {
        account: nested_ata,
        destination: wallet,
        authority: owner_ata,
        token_program: nested_token_program.address(),
    }
    .invoke_signed(&[Signer::from(&seeds)])
}
