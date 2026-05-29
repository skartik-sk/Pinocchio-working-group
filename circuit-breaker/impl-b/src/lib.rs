#![no_std]
#![no_main]

use pinocchio::entrypoint;
use pinocchio::pubkey::Pubkey;

pub mod state;
pub mod instructions;

use instructions::*;

entrypoint!(process_instruction);

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &mut [pinocchio::AccountInfo],
    instruction_data: &[u8],
) -> pinocchio::ProgramResult {
    let disc = *instruction_data.first().ok_or(pinocchio::ProgramError::InvalidInstruction)?;

    match disc {
        instructions::swap::INIT_DISC => instructions::swap::process_init(program_id, accounts, instruction_data),
        instructions::swap::SWAP_DISC => instructions::swap::process_swap(program_id, accounts, instruction_data),
        instructions::liquidity::DISC => instructions::liquidity::process_add(program_id, accounts, instruction_data),
        instructions::admin::PAUSE_DISC => instructions::admin::process_pause(program_id, accounts, instruction_data),
        instructions::admin::UPDATE_CB_DISC => instructions::admin::process_update_cb(program_id, accounts, instruction_data),
        instructions::admin::EMERGENCY_DISC => instructions::admin::process_emergency(program_id, accounts, instruction_data),
        _ => Err(pinocchio::ProgramError::InvalidInstruction),
    }
}
