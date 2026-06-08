// Tests for recover_nested program using Mollusk
// Follows https://solana.com/docs/programs/testing/mollusk

use {
    mollusk_svm::{Mollusk, result::Check},
    mollusk_svm_programs_token::{token, token2022},
    solana_account::Account,
    solana_address::Address,
    solana_instruction::{AccountMeta, Instruction},
    solana_program_error::ProgramError,
    solana_program_option::COption,
    solana_program_pack::Pack,
    solana_rent::Rent,
    spl_token_interface::state::{Account as TokenAccount, AccountState, Mint},
    std::path::PathBuf,
};

const PROGRAM_ID: Address = Address::new_from_array([1u8; 32]);
const SYSTEM_PROGRAM: Address = Address::new_from_array([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0,
]);

const TEST_MINT_AMOUNT: u64 = 100;

fn setup_mollusk() -> Mollusk {
    let mut mollusk = Mollusk::new(&PROGRAM_ID, "target/deploy/recover");

    let t22_elf_path = PathBuf::from(
        "/Users/singupallikartik/Developer/pinocchio-working-group/p-ata/Recover/benches/programs/spl_token_2022.so",
    );
    let t22_elf = mollusk_svm::file::read_file(t22_elf_path);
    mollusk.add_program_with_loader_and_elf(
        &spl_token_2022_interface::id(),
        &mollusk_svm::program::loader_keys::LOADER_V3,
        &t22_elf,
    );

    let t_elf_path = PathBuf::from(
        "/Users/singupallikartik/Developer/pinocchio-working-group/p-ata/Recover/benches/programs/pinocchio_token_program.so",
    );
    let t_elf = mollusk_svm::file::read_file(t_elf_path);
    mollusk.add_program_with_loader_and_elf(
        &spl_token_interface::id(),
        &mollusk_svm::program::loader_keys::LOADER_V3,
        &t_elf,
    );

    mollusk
}

/// Derive the ATA PDA using the same seeds as the program:
/// [wallet, mint, token_program]
fn derive_ata(wallet: &Address, mint: &Address, token_program: &Address) -> Address {
    Address::derive_program_address(
        &[
            wallet.as_ref(),
            mint.as_ref(),
            token_program.as_ref(),
        ],
        &PROGRAM_ID,
    )
    .unwrap()
    .0
}

fn make_mint(token_program: &Address) -> (Address, Account) {
    let mint = Address::new_unique();
    let mut data = vec![0u8; Mint::LEN];
    Mint::pack(
        Mint {
            mint_authority: COption::None,
            supply: 1_000_000,
            decimals: 9,
            is_initialized: true,
            freeze_authority: COption::None,
        },
        &mut data,
    )
    .unwrap();
    let rent = Rent::default();
    (
        mint,
        Account {
            lamports: rent.minimum_balance(Mint::LEN),
            data,
            owner: *token_program,
            executable: false,
            rent_epoch: 0,
        },
    )
}

fn make_token_account(
    token_program: &Address,
    mint: Address,
    owner: Address,
    amount: u64,
) -> Account {
    let rent = Rent::default();
    let mut data = vec![0u8; TokenAccount::LEN];
    TokenAccount::pack(
        TokenAccount {
            mint,
            owner,
            amount,
            delegate: COption::None,
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 0,
            close_authority: COption::None,
        },
        &mut data,
    )
    .unwrap();
    Account {
        lamports: rent.minimum_balance(TokenAccount::LEN),
        data,
        owner: *token_program,
        executable: false,
        rent_epoch: 0,
    }
}

fn system_account() -> (Address, Account) {
    mollusk_svm::program::keyed_account_for_system_program()
}

fn token_account_program() -> (Address, Account) {
    token::keyed_account()
}

fn token2022_account_program() -> (Address, Account) {
    token2022::keyed_account()
}

/// Build the recover_nested instruction.
///
/// Accounts:
/// 0. nested_ata          (writable)
/// 1. nested_token_mint   (readonly)
/// 2. destination_ata     (writable)
/// 3. owner_ata           (writable) — must be writable for CPI signing
/// 4. owner_token_mint    (readonly)
/// 5. wallet              (signer, writable)
/// 6. owner_token_program (readonly)
/// 7. nested_token_program(readonly, optional — required if different from owner)
fn build_recover_ix(
    nested_ata: &Address,
    nested_token_mint: &Address,
    destination_ata: &Address,
    owner_ata: &Address,
    owner_token_mint: &Address,
    wallet: &Address,
    owner_token_program: &Address,
    nested_token_program: &Address,
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new(*nested_ata, false),
        AccountMeta::new_readonly(*nested_token_mint, false),
        AccountMeta::new(*destination_ata, false),
        AccountMeta::new(*owner_ata, false),
        AccountMeta::new_readonly(*owner_token_mint, false),
        AccountMeta::new(*wallet, true),
        AccountMeta::new_readonly(*owner_token_program, false),
    ];

    // Only include the nested token program account when it differs from the
    // owner token program (the program uses the owner one as default).
    if nested_token_program != owner_token_program {
        accounts.push(AccountMeta::new_readonly(*nested_token_program, false));
    }

    Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data: vec![], // No instruction data needed
    }
}

// ─── Helper: set up a full nested-ATA scenario ─────────────────────────
//
// wallet ──owns──> owner_ata (for owner_mint, via owner_token_program)
// owner_ata ──owns──> nested_ata (for nested_mint, via nested_token_program)
// wallet ──owns──> destination_ata (for nested_mint, via nested_token_program)
//
// The nested_ata holds `TEST_MINT_AMOUNT` tokens that should be recovered.

struct RecoverSetup {
    wallet: Address,
    owner_mint: Address,
    nested_mint: Address,
    owner_ata: Address,
    nested_ata: Address,
    destination_ata: Address,
    nested_ata_lamports: u64,
}

fn setup_recover_scenario(
    owner_token_program: &Address,
    nested_token_program: &Address,
) -> (RecoverSetup, Vec<(Address, Account)>) {
    let wallet = Address::new_unique();
    let wallet_account = Account {
        lamports: 1_000_000_000,
        data: vec![],
        owner: SYSTEM_PROGRAM,
        executable: false,
        rent_epoch: 0,
    };

    // Owner side
    let (owner_mint, owner_mint_account) = make_mint(owner_token_program);
    let owner_ata = derive_ata(&wallet, &owner_mint, owner_token_program);
    let owner_ata_account = make_token_account(owner_token_program, owner_mint, wallet, 0);

    // Nested side
    let (nested_mint, nested_mint_account) = make_mint(nested_token_program);
    let nested_ata = derive_ata(&owner_ata, &nested_mint, nested_token_program);
    let nested_ata_account =
        make_token_account(nested_token_program, nested_mint, owner_ata, TEST_MINT_AMOUNT);

    // Destination: wallet's correct ATA for the nested mint
    let destination_ata = derive_ata(&wallet, &nested_mint, nested_token_program);
    let destination_ata_account =
        make_token_account(nested_token_program, nested_mint, wallet, 0);

    let nested_ata_lamports = nested_ata_account.lamports;

    let mut accounts: Vec<(Address, Account)> = vec![
        (nested_ata, nested_ata_account),
        (nested_mint, nested_mint_account),
        (destination_ata, destination_ata_account),
        (owner_ata, owner_ata_account),
        (owner_mint, owner_mint_account),
        (wallet, wallet_account),
    ];

    // Add the owner token program account
    let (spl_token_prog, spl_token_prog_acc) = token_account_program();
    let (t22_token_prog, t22_token_prog_acc) = token2022_account_program();

    if *owner_token_program == spl_token_interface::id() {
        accounts.push((spl_token_prog, spl_token_prog_acc.clone()));
    } else {
        accounts.push((t22_token_prog, t22_token_prog_acc.clone()));
    }

    // Add the nested token program account if different from owner
    if nested_token_program != owner_token_program {
        if *nested_token_program == spl_token_interface::id() {
            accounts.push((spl_token_prog, spl_token_prog_acc));
        } else {
            accounts.push((t22_token_prog, t22_token_prog_acc));
        }
    }

    let setup = RecoverSetup {
        wallet,
        owner_mint,
        nested_mint,
        owner_ata,
        nested_ata,
        destination_ata,
        nested_ata_lamports,
    };

    (setup, accounts)
}

// ─── TEST: Success — both SPL Token ─────────────────────────────────

#[test]
fn test_success_spl_token_both() {
    let mollusk = setup_mollusk();
    let owner_tp = spl_token_interface::id();
    let nested_tp = spl_token_interface::id();

    let (setup, accounts) = setup_recover_scenario(&owner_tp, &nested_tp);
    let wallet_lamports_before = accounts.iter().find(|(a, _)| *a == setup.wallet).unwrap().1.lamports;

    let ix = build_recover_ix(
        &setup.nested_ata,
        &setup.nested_mint,
        &setup.destination_ata,
        &setup.owner_ata,
        &setup.owner_mint,
        &setup.wallet,
        &owner_tp,
        &nested_tp,
    );

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[
            Check::success(),
            Check::account(&setup.wallet)
                .lamports(wallet_lamports_before + setup.nested_ata_lamports)
                .build(),
            Check::account(&setup.nested_ata).lamports(0).build(),
            Check::account(&setup.nested_ata).closed().build(),
        ],
    );
}

// ─── TEST: Success — both Token-2022 ─────────────────────────────────

#[test]
fn test_success_token2022_both() {
    let mollusk = setup_mollusk();
    let owner_tp = spl_token_2022_interface::id();
    let nested_tp = spl_token_2022_interface::id();

    let (setup, accounts) = setup_recover_scenario(&owner_tp, &nested_tp);
    let wallet_lamports_before = accounts.iter().find(|(a, _)| *a == setup.wallet).unwrap().1.lamports;

    let ix = build_recover_ix(
        &setup.nested_ata,
        &setup.nested_mint,
        &setup.destination_ata,
        &setup.owner_ata,
        &setup.owner_mint,
        &setup.wallet,
        &owner_tp,
        &nested_tp,
    );

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[
            Check::success(),
            Check::account(&setup.wallet)
                .lamports(wallet_lamports_before + setup.nested_ata_lamports)
                .build(),
            Check::account(&setup.nested_ata).lamports(0).build(),
            Check::account(&setup.nested_ata).closed().build(),
        ],
    );
}

// ─── TEST: Success — owner SPL Token, nested Token-2022 ──────────────

#[test]
fn test_success_owner_spl_nested_t22() {
    let mollusk = setup_mollusk();
    let owner_tp = spl_token_interface::id();
    let nested_tp = spl_token_2022_interface::id();

    let (setup, accounts) = setup_recover_scenario(&owner_tp, &nested_tp);
    let wallet_lamports_before = accounts.iter().find(|(a, _)| *a == setup.wallet).unwrap().1.lamports;

    let ix = build_recover_ix(
        &setup.nested_ata,
        &setup.nested_mint,
        &setup.destination_ata,
        &setup.owner_ata,
        &setup.owner_mint,
        &setup.wallet,
        &owner_tp,
        &nested_tp,
    );

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[
            Check::success(),
            Check::account(&setup.wallet)
                .lamports(wallet_lamports_before + setup.nested_ata_lamports)
                .build(),
            Check::account(&setup.nested_ata).lamports(0).build(),
            Check::account(&setup.nested_ata).closed().build(),
        ],
    );
}

// ─── TEST: Success — owner Token-2022, nested SPL Token ──────────────

#[test]
fn test_success_owner_t22_nested_spl() {
    let mollusk = setup_mollusk();
    let owner_tp = spl_token_2022_interface::id();
    let nested_tp = spl_token_interface::id();

    let (setup, accounts) = setup_recover_scenario(&owner_tp, &nested_tp);
    let wallet_lamports_before = accounts.iter().find(|(a, _)| *a == setup.wallet).unwrap().1.lamports;

    let ix = build_recover_ix(
        &setup.nested_ata,
        &setup.nested_mint,
        &setup.destination_ata,
        &setup.owner_ata,
        &setup.owner_mint,
        &setup.wallet,
        &owner_tp,
        &nested_tp,
    );

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[
            Check::success(),
            Check::account(&setup.wallet)
                .lamports(wallet_lamports_before + setup.nested_ata_lamports)
                .build(),
            Check::account(&setup.nested_ata).lamports(0).build(),
            Check::account(&setup.nested_ata).closed().build(),
        ],
    );
}

// ─── TEST: Fail — wallet not a signer ──────────────────────────────────

#[test]
fn test_fail_wallet_not_signer() {
    let mollusk = setup_mollusk();
    let owner_tp = spl_token_interface::id();
    let nested_tp = spl_token_interface::id();

    let (setup, accounts) = setup_recover_scenario(&owner_tp, &nested_tp);

    // Build instruction WITHOUT signer on wallet
    let mut ix = build_recover_ix(
        &setup.nested_ata,
        &setup.nested_mint,
        &setup.destination_ata,
        &setup.owner_ata,
        &setup.owner_mint,
        &setup.wallet,
        &owner_tp,
        &nested_tp,
    );
    // Remove signer flag from wallet (index 5)
    ix.accounts[5].is_signer = false;

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[Check::err(ProgramError::MissingRequiredSignature)],
    );
}

// ─── TEST: Fail — wrong nested_ata address (invalid seeds) ────────────

#[test]
fn test_fail_wrong_nested_ata_address() {
    let mollusk = setup_mollusk();
    let owner_tp = spl_token_interface::id();
    let nested_tp = spl_token_interface::id();

    let (setup, mut accounts) = setup_recover_scenario(&owner_tp, &nested_tp);

    // Use a random wrong address for nested_ata
    let wrong_nested_ata = Address::new_unique();
    let wrong_account = make_token_account(&nested_tp, setup.nested_mint, setup.owner_ata, TEST_MINT_AMOUNT);

    // Replace the nested_ata in accounts
    for (addr, acc) in accounts.iter_mut() {
        if *addr == setup.nested_ata {
            *addr = wrong_nested_ata;
            *acc = wrong_account;
            break;
        }
    }

    let ix = build_recover_ix(
        &wrong_nested_ata,
        &setup.nested_mint,
        &setup.destination_ata,
        &setup.owner_ata,
        &setup.owner_mint,
        &setup.wallet,
        &owner_tp,
        &nested_tp,
    );

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[Check::err(ProgramError::InvalidSeeds)],
    );
}

// ─── TEST: Fail — wrong destination_ata address (invalid seeds) ───────

#[test]
fn test_fail_wrong_destination_ata_address() {
    let mollusk = setup_mollusk();
    let owner_tp = spl_token_interface::id();
    let nested_tp = spl_token_interface::id();

    let (setup, mut accounts) = setup_recover_scenario(&owner_tp, &nested_tp);

    // Use a random wrong address for destination_ata
    let wrong_dest_ata = Address::new_unique();
    let wrong_account = make_token_account(&nested_tp, setup.nested_mint, setup.wallet, 0);

    // Replace the destination_ata in accounts
    for (addr, acc) in accounts.iter_mut() {
        if *addr == setup.destination_ata {
            *addr = wrong_dest_ata;
            *acc = wrong_account;
            break;
        }
    }

    let ix = build_recover_ix(
        &setup.nested_ata,
        &setup.nested_mint,
        &wrong_dest_ata,
        &setup.owner_ata,
        &setup.owner_mint,
        &setup.wallet,
        &owner_tp,
        &nested_tp,
    );

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[Check::err(ProgramError::InvalidSeeds)],
    );
}

// ─── TEST: Fail — wrong owner_ata address (invalid seeds) ─────────────

#[test]
fn test_fail_wrong_owner_ata_address() {
    let mollusk = setup_mollusk();
    let owner_tp = spl_token_interface::id();
    let nested_tp = spl_token_interface::id();

    let (setup, mut accounts) = setup_recover_scenario(&owner_tp, &nested_tp);

    // Use a random wrong address for owner_ata
    let wrong_owner_ata = Address::new_unique();
    let wrong_account = make_token_account(&owner_tp, setup.owner_mint, setup.wallet, 0);

    // Replace the owner_ata in accounts
    for (addr, acc) in accounts.iter_mut() {
        if *addr == setup.owner_ata {
            *addr = wrong_owner_ata;
            *acc = wrong_account;
            break;
        }
    }

    let ix = build_recover_ix(
        &setup.nested_ata,
        &setup.nested_mint,
        &setup.destination_ata,
        &wrong_owner_ata,
        &setup.owner_mint,
        &setup.wallet,
        &owner_tp,
        &nested_tp,
    );

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[Check::err(ProgramError::InvalidSeeds)],
    );
}

// ─── TEST: Fail — nested_ata not owned by owner_ata (IllegalOwner) ───

#[test]
fn test_fail_nested_ata_wrong_owner() {
    let mollusk = setup_mollusk();
    let owner_tp = spl_token_interface::id();
    let nested_tp = spl_token_interface::id();

    let (setup, mut accounts) = setup_recover_scenario(&owner_tp, &nested_tp);

    // Replace nested_ata with one owned by a random address instead of owner_ata
    let random_owner = Address::new_unique();
    let wrong_nested_account = make_token_account(&nested_tp, setup.nested_mint, random_owner, TEST_MINT_AMOUNT);

    for (addr, acc) in accounts.iter_mut() {
        if *addr == setup.nested_ata {
            *acc = wrong_nested_account;
            break;
        }
    }

    let ix = build_recover_ix(
        &setup.nested_ata,
        &setup.nested_mint,
        &setup.destination_ata,
        &setup.owner_ata,
        &setup.owner_mint,
        &setup.wallet,
        &owner_tp,
        &nested_tp,
    );

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[Check::err(ProgramError::IllegalOwner)],
    );
}

// ─── TEST: Fail — owner_ata not owned by wallet (IllegalOwner) ────────

#[test]
fn test_fail_owner_ata_wrong_owner() {
    let mollusk = setup_mollusk();
    let owner_tp = spl_token_interface::id();
    let nested_tp = spl_token_interface::id();

    let (setup, mut accounts) = setup_recover_scenario(&owner_tp, &nested_tp);

    // Replace owner_ata with one owned by a random address instead of wallet
    let random_owner = Address::new_unique();
    let wrong_owner_account = make_token_account(&owner_tp, setup.owner_mint, random_owner, 0);

    for (addr, acc) in accounts.iter_mut() {
        if *addr == setup.owner_ata {
            *acc = wrong_owner_account;
            break;
        }
    }

    let ix = build_recover_ix(
        &setup.nested_ata,
        &setup.nested_mint,
        &setup.destination_ata,
        &setup.owner_ata,
        &setup.owner_mint,
        &setup.wallet,
        &owner_tp,
        &nested_tp,
    );

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[Check::err(ProgramError::IllegalOwner)],
    );
}

// ─── TEST: Fail — missing nested_token_program when programs differ ────

#[test]
fn test_fail_missing_nested_token_program() {
    let mollusk = setup_mollusk();
    let owner_tp = spl_token_interface::id();
    let nested_tp = spl_token_2022_interface::id();

    let (setup, accounts) = setup_recover_scenario(&owner_tp, &nested_tp);

    // Build the instruction with both programs, then truncate to remove
    // the 8th account (nested_token_program)
    let mut ix = build_recover_ix(
        &setup.nested_ata,
        &setup.nested_mint,
        &setup.destination_ata,
        &setup.owner_ata,
        &setup.owner_mint,
        &setup.wallet,
        &owner_tp,
        &nested_tp,
    );

    // Remove the last account (nested_token_program) to simulate missing account
    ix.accounts.truncate(7);

    // Also need to remove the last account from the accounts list
    let mut truncated_accounts = accounts.clone();
    // Remove the nested token program entry (last one)
    truncated_accounts.pop();

    mollusk.process_and_validate_instruction(
        &ix,
        &truncated_accounts,
        &[Check::err(ProgramError::InvalidSeeds)],
    );
}
