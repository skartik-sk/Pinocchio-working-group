// Tests for create-idempotent program using Mollusk
// Follows https://solana.com/docs/programs/testing/mollusk

use {
    mollusk_svm::{Mollusk, result::Check},
    mollusk_svm_programs_token::{token, token2022},
    solana_account::Account,
    solana_address::Address,
    solana_instruction::{AccountMeta, Instruction},
    solana_program_option::COption,
    solana_program_pack::Pack,
    solana_rent::Rent,
    spl_token_interface::state::{Account as TokenAccount, AccountState, Mint}, std::path::PathBuf,
};

const PROGRAM_ID: Address = Address::new_from_array([1u8; 32]);
const SYSTEM_PROGRAM: Address = Address::new_from_array([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
]);

fn setup_mollusk() -> Mollusk {
    let mut mollusk = Mollusk::new(&PROGRAM_ID, "target/deploy/create_idempotent");

    let t22_elf_path = PathBuf::from("/Users/singupallikartik/Developer/pinocchio-working-group/p-ata/create-idempotent/benches/programs/spl_token_2022.so");
        let t22_elf = mollusk_svm::file::read_file(t22_elf_path);
        mollusk.add_program_with_loader_and_elf(
            &spl_token_2022_interface::id(),
            &mollusk_svm::program::loader_keys::LOADER_V3,
            &t22_elf,
        );
        let t_elf_path = PathBuf::from("/Users/singupallikartik/Developer/pinocchio-working-group/p-ata/create-idempotent/benches/programs/pinocchio_token_program.so");
            let t_elf = mollusk_svm::file::read_file(t_elf_path);
            mollusk.add_program_with_loader_and_elf(
                &spl_token_interface::id(),
                &mollusk_svm::program::loader_keys::LOADER_V3,
                &t_elf,
            );
    // token::add_program(&mut mollusk);
    // token2022::add_program(&mut mollusk);
    mollusk
}

/// Derive the ATA PDA using the same seeds as the program:
/// [wallet, mint, token_program]
fn derive_ata(wallet: &Address, mint: &Address, token_program: &Address) -> Address {
    Address::derive_program_address(
        &[wallet.as_ref(), mint.as_ref(), token_program.as_ref()],
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

fn make_funder() -> (Address, Account) {
    (
        Address::new_unique(),
        Account {
            lamports: 10_000_000_000,
            data: vec![],
            owner: SYSTEM_PROGRAM,
            executable: false,
            rent_epoch: 0,
        },
    )
}

fn make_wallet() -> (Address, Account) {
    (
        Address::new_unique(),
        Account {
            lamports: 1_000_000_000,
            data: vec![],
            owner: SYSTEM_PROGRAM,
            executable: false,
            rent_epoch: 0,
        },
    )
}

fn build_instruction(
    funder: &Address,
    ata: &Address,
    wallet: &Address,
    mint: &Address,
    token_program: &Address,
) -> Instruction {
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*funder, true),                  // 0: funder
            AccountMeta::new(*ata, false),                    // 1: ATA
            AccountMeta::new_readonly(*wallet, false),        // 2: wallet
            AccountMeta::new_readonly(*mint, false),          // 3: mint
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false), // 4: system program
            AccountMeta::new_readonly(*token_program, false), // 5: token program
        ],
        data: vec![],
    }
}

fn system_account() -> (Address, Account) {
    mollusk_svm::program::keyed_account_for_system_program()
}

fn token_account() -> (Address, Account) {
    token::keyed_account()
}

fn token2022_account() -> (Address, Account) {
    token2022::keyed_account()
}

// ─── TEST: Create new ATA with SPL Token ────────────────────────────

#[test]
fn test_create_new_ata_spl_token() {
    let mollusk = setup_mollusk();
    let token_program = spl_token_interface::id();

    let (funder, funder_account) = make_funder();
    let (wallet, wallet_account) = make_wallet();
    let (mint, mint_account) = make_mint(&token_program);
    let ata = derive_ata(&wallet, &mint, &token_program);

    let accounts = vec![
        (funder, funder_account),
        (ata, Account::default()),
        (wallet, wallet_account),
        (mint, mint_account),
        system_account(),
        token_account(),
    ];

    let ix = build_instruction(&funder, &ata, &wallet, &mint, &token_program);

    let result = mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[
            Check::success(),
            Check::account(&ata)
                .owner(&token_program)
                .space(TokenAccount::LEN)
                .rent_exempt()
                .build(),
        ],
    );

    let ata_acc = result.get_account(&ata).unwrap();
    let token_acc = TokenAccount::unpack(&ata_acc.data).unwrap();
    assert_eq!(token_acc.mint, mint);
    assert_eq!(token_acc.owner, wallet);
    assert_eq!(token_acc.amount, 0);
}

// ─── TEST: Create new ATA with Token-2022 ────────────────────────────

#[test]
fn test_create_new_ata_token2022() {
    let mollusk = setup_mollusk();
    let token_program = spl_token_2022_interface::id();

    let (funder, funder_account) = make_funder();
    let (wallet, wallet_account) = make_wallet();
    let (mint, mint_account) = make_mint(&token_program);
    let ata = derive_ata(&wallet, &mint, &token_program);

    let accounts = vec![
        (funder, funder_account),
        (ata, Account::default()),
        (wallet, wallet_account),
        (mint, mint_account),
        system_account(),
        token2022_account(),
    ];

    let ix = build_instruction(&funder, &ata, &wallet, &mint, &token_program);

    let result = mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[
            Check::success(),
            Check::account(&ata)
                .owner(&token_program)
                .rent_exempt()
                .build(),
        ],
    );

    let ata_acc = result.get_account(&ata).unwrap();
    assert!(ata_acc.data.len() > TokenAccount::LEN);
}

// ─── TEST: Idempotent — existing ATA with correct owner/mint ──────────

#[test]
fn test_idempotent_existing_ata() {
    let mollusk = setup_mollusk();
    let token_program = spl_token_interface::id();

    let (funder, funder_account) = make_funder();
    let (wallet, wallet_account) = make_wallet();
    let (mint, mint_account) = make_mint(&token_program);
    let ata = derive_ata(&wallet, &mint, &token_program);

    let rent = Rent::default();
    let mut ata_data = vec![0u8; TokenAccount::LEN];
    TokenAccount::pack(
        TokenAccount {
            mint,
            owner: wallet,
            amount: 0,
            delegate: COption::None,
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 0,
            close_authority: COption::None,
        },
        &mut ata_data,
    )
    .unwrap();

    let existing_ata = Account {
        lamports: rent.minimum_balance(TokenAccount::LEN),
        data: ata_data,
        owner: token_program,
        executable: false,
        rent_epoch: 0,
    };

    let accounts = vec![
        (funder, funder_account),
        (ata, existing_ata),
        (wallet, wallet_account),
        (mint, mint_account),
        system_account(),
        token_account(),
    ];

    let ix = build_instruction(&funder, &ata, &wallet, &mint, &token_program);

    mollusk.process_and_validate_instruction(&ix, &accounts, &[Check::success()]);
}

// ─── TEST: Idempotent — existing Token-2022 ATA with correct owner/mint ─

#[test]
fn test_idempotent_existing_ata_token2022() {
    let mollusk = setup_mollusk();
    let token_program = spl_token_2022_interface::id();

    let (funder, funder_account) = make_funder();
    let (wallet, wallet_account) = make_wallet();
    let (mint, mint_account) = make_mint(&token_program);
    let ata = derive_ata(&wallet, &mint, &token_program);

    // Build a Token-2022 account: 165 (base) + 1 (account_type) + 4 (TLV header) = 170 bytes
    const T22_ACCOUNT_LEN: usize = 170;
    let rent = Rent::default();
    let mut ata_data = vec![0u8; T22_ACCOUNT_LEN];
    TokenAccount::pack(
        TokenAccount {
            mint,
            owner: wallet,
            amount: 0,
            delegate: COption::None,
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 0,
            close_authority: COption::None,
        },
        &mut ata_data[..TokenAccount::LEN],
    )
    .unwrap();
    // AccountType = 2 (Account)
    ata_data[165] = 2;
    // TLV: type = 7 (ImmutableOwner), length = 0
    ata_data[166] = 7;
    ata_data[167] = 0;
    ata_data[168] = 0;
    ata_data[169] = 0;

    let existing_ata = Account {
        lamports: rent.minimum_balance(T22_ACCOUNT_LEN),
        data: ata_data,
        owner: token_program,
        executable: false,
        rent_epoch: 0,
    };

    let accounts = vec![
        (funder, funder_account),
        (ata, existing_ata),
        (wallet, wallet_account),
        (mint, mint_account),
        system_account(),
        token2022_account(),
    ];

    let ix = build_instruction(&funder, &ata, &wallet, &mint, &token_program);

    mollusk.process_and_validate_instruction(&ix, &accounts, &[Check::success()]);
}

// ─── TEST: Wrong ATA address fails ──────────────────────────────────

#[test]
fn test_wrong_ata_address_fails() {
    let mollusk = setup_mollusk();
    let token_program = spl_token_interface::id();

    let (funder, funder_account) = make_funder();
    let (wallet, wallet_account) = make_wallet();
    let (mint, mint_account) = make_mint(&token_program);
    let wrong_ata = Address::new_unique();

    let accounts = vec![
        (funder, funder_account),
        (wrong_ata, Account::default()),
        (wallet, wallet_account),
        (mint, mint_account),
        system_account(),
        token_account(),
    ];

    let ix = build_instruction(&funder, &wrong_ata, &wallet, &mint, &token_program);

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[Check::err(solana_program_error::ProgramError::InvalidSeeds)],
    );
}

// ─── TEST: Existing ATA with wrong owner fails ───────────────────────

#[test]
fn test_existing_ata_wrong_owner_fails() {
    let mollusk = setup_mollusk();
    let token_program = spl_token_interface::id();

    let (funder, funder_account) = make_funder();
    let (wallet, wallet_account) = make_wallet();
    let (mint, mint_account) = make_mint(&token_program);
    let ata = derive_ata(&wallet, &mint, &token_program);
    let wrong_owner = Address::new_unique();

    let rent = Rent::default();
    let mut ata_data = vec![0u8; TokenAccount::LEN];
    TokenAccount::pack(
        TokenAccount {
            mint,
            owner: wrong_owner,
            amount: 0,
            delegate: COption::None,
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 0,
            close_authority: COption::None,
        },
        &mut ata_data,
    )
    .unwrap();

    let existing_ata = Account {
        lamports: rent.minimum_balance(TokenAccount::LEN),
        data: ata_data,
        owner: token_program,
        executable: false,
        rent_epoch: 0,
    };

    let accounts = vec![
        (funder, funder_account),
        (ata, existing_ata),
        (wallet, wallet_account),
        (mint, mint_account),
        system_account(),
        token_account(),
    ];

    let ix = build_instruction(&funder, &ata, &wallet, &mint, &token_program);

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[Check::err(solana_program_error::ProgramError::IllegalOwner)],
    );
}

// ─── TEST: Existing ATA with wrong mint fails ────────────────────────

#[test]
fn test_existing_ata_wrong_mint_fails() {
    let mollusk = setup_mollusk();
    let token_program = spl_token_interface::id();

    let (funder, funder_account) = make_funder();
    let (wallet, wallet_account) = make_wallet();
    let (mint, mint_account) = make_mint(&token_program);
    let ata = derive_ata(&wallet, &mint, &token_program);
    let wrong_mint = Address::new_unique();

    let rent = Rent::default();
    let mut ata_data = vec![0u8; TokenAccount::LEN];
    TokenAccount::pack(
        TokenAccount {
            mint: wrong_mint,
            owner: wallet,
            amount: 0,
            delegate: COption::None,
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 0,
            close_authority: COption::None,
        },
        &mut ata_data,
    )
    .unwrap();

    let existing_ata = Account {
        lamports: rent.minimum_balance(TokenAccount::LEN),
        data: ata_data,
        owner: token_program,
        executable: false,
        rent_epoch: 0,
    };

    let accounts = vec![
        (funder, funder_account),
        (ata, existing_ata),
        (wallet, wallet_account),
        (mint, mint_account),
        system_account(),
        token_account(),
    ];

    let ix = build_instruction(&funder, &ata, &wallet, &mint, &token_program);

    mollusk.process_and_validate_instruction(
        &ix,
        &accounts,
        &[Check::err(
            solana_program_error::ProgramError::InvalidAccountData,
        )],
    );
}
