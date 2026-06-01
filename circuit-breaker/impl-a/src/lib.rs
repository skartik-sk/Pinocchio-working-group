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

#[cfg(test)]
mod tests {
    use crate::state::*;

    #[test]
    fn test_escrow_layout() {
        assert_eq!(Escrow::LEN, 113);
        assert_eq!(Escrow::OFFSET_MAKER, 0);
        assert_eq!(Escrow::OFFSET_MINT_A, 32);
        assert_eq!(Escrow::OFFSET_MINT_B, 64);
        assert_eq!(Escrow::OFFSET_AMOUNT, 96);
        assert_eq!(Escrow::OFFSET_EXPIRY, 104);
        assert_eq!(Escrow::OFFSET_BUMP, 112);
    }

    #[test]
    fn test_cb_layout() {
        assert_eq!(CircuitBreaker::LEN, 67);
        assert_eq!(CircuitBreaker::OFFSET_AUTHORITY, 0);
        assert_eq!(CircuitBreaker::OFFSET_PAUSED, 32);
        assert_eq!(CircuitBreaker::OFFSET_WINDOW_SEC, 33);
        assert_eq!(CircuitBreaker::OFFSET_THRESHOLD_TYPE, 41);
        assert_eq!(CircuitBreaker::OFFSET_THRESHOLD, 42);
        assert_eq!(CircuitBreaker::OFFSET_LAST_VALUE, 50);
        assert_eq!(CircuitBreaker::OFFSET_LAST_TS, 58);
        assert_eq!(CircuitBreaker::OFFSET_BUMP, 66);
    }
}
