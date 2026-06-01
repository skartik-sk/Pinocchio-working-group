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
        instructions::swap::INIT_DISC => instructions::swap::process_init(program_id, accounts, instruction_data),
        instructions::swap::SWAP_DISC => instructions::swap::process_swap(program_id, accounts, instruction_data),
        instructions::liquidity::DISC => instructions::liquidity::process_add(program_id, accounts, instruction_data),
        instructions::admin::PAUSE_DISC => instructions::admin::process_pause(program_id, accounts, instruction_data),
        instructions::admin::UPDATE_CB_DISC => instructions::admin::process_update_cb(program_id, accounts, instruction_data),
        instructions::admin::EMERGENCY_DISC => instructions::admin::process_emergency(program_id, accounts, instruction_data),
        _ => Err(pinocchio::error::ProgramError::InvalidInstructionData),
    }
}

#[cfg(test)]
mod tests {
    use crate::state::*;

    #[test]
    fn test_pool_layout() {
        assert_eq!(PoolState::LEN, 238);
        assert_eq!(PoolState::OFFSET_AUTHORITY, 0);
        assert_eq!(PoolState::OFFSET_MINT_A, 32);
        assert_eq!(PoolState::OFFSET_MINT_B, 64);
        assert_eq!(PoolState::OFFSET_RESERVE_A, 96);
        assert_eq!(PoolState::OFFSET_RESERVE_B, 128);
        assert_eq!(PoolState::OFFSET_LP_MINT, 160);
        assert_eq!(PoolState::OFFSET_AMP, 192);
        assert_eq!(PoolState::OFFSET_TRADE_FEE, 200);
        assert_eq!(PoolState::OFFSET_ADMIN_FEE, 202);
        assert_eq!(PoolState::OFFSET_PAUSED, 204);
        assert_eq!(PoolState::OFFSET_CB_WINDOW, 205);
        assert_eq!(PoolState::OFFSET_CB_THRESH, 213);
        assert_eq!(PoolState::OFFSET_CB_LAST_VAL, 221);
        assert_eq!(PoolState::OFFSET_CB_LAST_TS, 229);
        assert_eq!(PoolState::OFFSET_BUMP, 237);
    }
}
