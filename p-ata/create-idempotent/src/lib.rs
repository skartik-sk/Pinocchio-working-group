use pinocchio::{
    AccountView, Address, ProgramResult,
    cpi::Signer,
    entrypoint,
    error::ProgramError,
    instruction::seeds,
};

use pinocchio_system::instructions::CreateAccountAllowPrefund;
use pinocchio_token::instructions::InitializeAccount3;
use pinocchio_token_2022::instructions::{
    InitializeAccount3 as InitializeAccount3Token2022, InitializeImmutableOwner,
};
use pinocchio_token_2022::state::{
    Account, ExtensionType, Mint, StateWithExtensions, try_calculate_account_len,
};
mod batch;

use crate::batch::batch_init_and_lock_owner;

entrypoint!(process_instruction);

pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    _instruction_data: &[u8],
) -> ProgramResult {
    let [
        funder_info,
        associated_token_account_info,
        wallet_account_info,
        spl_token_mint_info,
        _system_program_info,
        spl_token_program_info,
        _other @ ..,
    ] = accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let Some((derive_associated_token_account, bump_seed)) = Address::derive_program_address(
        &[
            &wallet_account_info.address().to_bytes(),
            &spl_token_mint_info.address().to_bytes(),
            &spl_token_program_info.address().to_bytes(),
        ],
        program_id,
    ) else {
        return Err(ProgramError::InvalidAccountData);
    };
    if &derive_associated_token_account != associated_token_account_info.address() {
        return Err(ProgramError::InvalidSeeds);
    }

    // for normal create fn wrap it into if block
    if associated_token_account_info.owner() == spl_token_program_info.address() {
        let ata_data = associated_token_account_info.try_borrow()?;
        if let Ok(associated_token_account_bytes) =
            StateWithExtensions::<Account>::from_bytes(&ata_data)
        {
            if associated_token_account_bytes.base.owner() != wallet_account_info.address() {
                let error = ProgramError::IllegalOwner;
                return Err(error.into());
            }
            if associated_token_account_bytes.base.mint() != spl_token_mint_info.address() {
                return Err(ProgramError::InvalidAccountData);
            }
            return Ok(());
        }
    }

    if *associated_token_account_info.owner() != pinocchio_system::id() {
        return Err(ProgramError::IllegalOwner);
    }


    let is_spl_token = *spl_token_program_info.address() == pinocchio_token::ID;
    let account_len = if is_spl_token {
        Account::BASE_LEN as u64
    } else {
        let mint_data = spl_token_mint_info.try_borrow()?;

        if mint_data.len() == Mint::BASE_LEN {
            TOKEN_2022_BASE_ACCOUNT_DATA_SIZE
        } else {

            let mut extensions = [ExtensionType::ImmutableOwner; 8];
            let mut ext_count: usize = 1;

            let mut offset = Mint::BASE_LEN + ACCOUNT_TYPE_SIZE;

            while offset + TLV_HEADER_LEN <= mint_data.len() {
                let ext_type =
                    u16::from_le_bytes([mint_data[offset], mint_data[offset + 1]]);
                let ext_len =
                    u16::from_le_bytes([mint_data[offset + 2], mint_data[offset + 3]]) as usize;

                if ext_type == 0 || ext_type > 28 {
                    break;
                }

                let ext = match ext_type {
                    1 => Some(ExtensionType::TransferFeeAmount),
                    9 => Some(ExtensionType::NonTransferableAccount),
                    14 => Some(ExtensionType::TransferHookAccount),
                    26 => Some(ExtensionType::PausableAccount),
                    _ => None,
                };

                if let Some(e) = ext {
                    extensions[ext_count] = e;
                    ext_count += 1;
                }

                offset += TLV_HEADER_LEN + ext_len;
            }

            try_calculate_account_len::<Account>(&extensions[..ext_count])? as u64
        }
    };

    let bump_ref = &[bump_seed];
    let seeds = seeds!(
        wallet_account_info.address().as_ref(),
        spl_token_mint_info.address().as_ref(),
        spl_token_program_info.address().as_ref(),
        bump_ref
    );
    let signer = Signer::from(&seeds);
    CreateAccountAllowPrefund::with_minimum_balance(funder_info,
    associated_token_account_info,
    account_len,
    spl_token_program_info.address(),
    None)?
    .invoke_signed(&[signer])?;

    if !is_spl_token{
        batch_init_and_lock_owner( spl_token_program_info.address(), associated_token_account_info, spl_token_mint_info, wallet_account_info.address())?;

    } else {
        InitializeAccount3::new(
            associated_token_account_info,
            spl_token_mint_info,
            wallet_account_info.address(),
        )
        .invoke()?;
    }
    Ok(())
}

const ACCOUNT_TYPE_SIZE: usize = 1;
const TLV_HEADER_LEN: usize = 4;
const TOKEN_2022_BASE_ACCOUNT_DATA_SIZE: u64 =
    Account::BASE_LEN as u64 + ACCOUNT_TYPE_SIZE as u64 + TLV_HEADER_LEN as u64;
