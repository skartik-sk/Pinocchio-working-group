#![cfg_attr(feature = "bpf-entrypoint", no_std)]
#![cfg_attr(feature = "bpf-entrypoint", no_main)]

pub mod state;

#[cfg(feature = "bpf-entrypoint")]
pub mod instructions;

#[cfg(feature = "bpf-entrypoint")]
use pinocchio::{
    entrypoint,
    AccountView,
    Address,
    ProgramResult,
    nostd_panic_handler,
};

#[cfg(feature = "bpf-entrypoint")]
nostd_panic_handler!();

#[cfg(feature = "bpf-entrypoint")]
entrypoint!(process_instruction);

#[cfg(feature = "bpf-entrypoint")]
pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let disc = *instruction_data.first().ok_or(pinocchio::error::ProgramError::InvalidInstructionData)?;

    match disc {
        instructions::make::DISC => instructions::make::process(program_id, accounts, instruction_data),
        instructions::take::DISC => instructions::take::process(program_id, accounts, instruction_data),
        instructions::refund::DISC => instructions::refund::process(program_id, accounts, instruction_data),
        instructions::circuit_breaker::INIT_DISC => instructions::circuit_breaker::process_init(program_id, accounts, instruction_data),
        instructions::circuit_breaker::UPDATE_DISC => instructions::circuit_breaker::process_update(program_id, accounts, instruction_data),
        instructions::circuit_breaker::PAUSE_DISC => instructions::circuit_breaker::process_pause(program_id, accounts, instruction_data),
        _ => Err(pinocchio::error::ProgramError::InvalidInstructionData),
    }
}


