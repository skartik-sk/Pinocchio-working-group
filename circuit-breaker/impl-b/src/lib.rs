#![no_std]
#![no_main]

use pinocchio::{
    entrypoint,
    AccountView,
    Address,
    ProgramResult,
    nostd_panic_handler,
};

nostd_panic_handler!();

pub mod state;
pub mod instructions;

entrypoint!(process_instruction);

pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let disc = *instruction_data.first().ok_or(pinocchio::error::ProgramError::InvalidInstructionData)?;

    match disc {
        instructions::swap::INIT_DISC => instructions::swap::process_init(program_id, accounts, instruction_data),
        instructions::swap::SWAP_DISC => instructions::swap::process_swap(program_id, accounts, instruction_data),
        instructions::liquidity::DISC => instructions::liquidity::process_add(program_id, accounts, instruction_data),
        instructions::admin::PAUSE_DISC => instructions::admin::process_pause(program_id, accounts, instruction_data),
        instructions::admin::UPDATE_CB_DISC => instructions::admin::process_update_cb(program_id, accounts, instruction_data),
        instructions::admin::EMERGENCY_DISC => instructions::admin::process_emergency(program_id, accounts, instruction_data),
        _ => Err(pinocchio::error::ProgramError::InvalidInstructionData),
    }
}
