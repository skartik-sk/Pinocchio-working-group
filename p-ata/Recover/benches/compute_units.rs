// Compute unit benchmarks for recover_nested program
// Follows https://solana.com/docs/programs/testing/mollusk#compute-unit-benchmarking

use {
    mollusk_svm::Mollusk,
    mollusk_svm_bencher::MolluskComputeUnitBencher,
    mollusk_svm_programs_token::{token, token2022},
    solana_account::Account,
    solana_address::Address,
    solana_instruction::{AccountMeta, Instruction},
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

/// Build the recover_nested instruction with all accounts.
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

    if nested_token_program != owner_token_program {
        accounts.push(AccountMeta::new_readonly(*nested_token_program, false));
    }

    Instruction {
        program_id: PROGRAM_ID,
        accounts,
        data: vec![],
    }
}

/// Set up a full recover_nested scenario and return (instruction, accounts).
fn recover_nested_case(
    owner_token_program_id: Address,
    nested_token_program_id: Address,
    spl_token_account: &(Address, Account),
    t22_account: &(Address, Account),
) -> (Instruction, Vec<(Address, Account)>) {
    let wallet = Address::new_unique();

    // Owner side
    let (owner_mint, owner_mint_account) = make_mint(&owner_token_program_id);
    let owner_ata = derive_ata(&wallet, &owner_mint, &owner_token_program_id);
    let owner_ata_account = make_token_account(
        &owner_token_program_id,
        owner_mint,
        wallet,
        0,
    );

    // Nested side
    let (nested_mint, nested_mint_account) = make_mint(&nested_token_program_id);
    let nested_ata = derive_ata(&owner_ata, &nested_mint, &nested_token_program_id);
    let nested_ata_account = make_token_account(
        &nested_token_program_id,
        nested_mint,
        owner_ata,
        TEST_MINT_AMOUNT,
    );

    // Destination: wallet's correct ATA for the nested mint
    let destination_ata = derive_ata(&wallet, &nested_mint, &nested_token_program_id);
    let destination_ata_account = make_token_account(
        &nested_token_program_id,
        nested_mint,
        wallet,
        0,
    );

    let mut accounts: Vec<(Address, Account)> = vec![
        (nested_ata, nested_ata_account),
        (nested_mint, nested_mint_account),
        (destination_ata, destination_ata_account),
        (owner_ata, owner_ata_account),
        (owner_mint, owner_mint_account),
        (
            wallet,
            Account {
                lamports: 1_000_000_000,
                data: vec![],
                owner: SYSTEM_PROGRAM,
                executable: false,
                rent_epoch: 0,
            },
        ),
    ];

    // Owner token program account
    if owner_token_program_id == spl_token_interface::id() {
        accounts.push(spl_token_account.clone());
    } else {
        accounts.push(t22_account.clone());
    }

    // Nested token program account (only if different from owner)
    if owner_token_program_id != nested_token_program_id {
        if nested_token_program_id == spl_token_interface::id() {
            accounts.push(spl_token_account.clone());
        } else {
            accounts.push(t22_account.clone());
        }
    }

    let ix = build_recover_ix(
        &nested_ata,
        &nested_mint,
        &destination_ata,
        &owner_ata,
        &owner_mint,
        &wallet,
        &owner_token_program_id,
        &nested_token_program_id,
    );

    (ix, accounts)
}

fn main() {
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

    let spl_token_acc = token::keyed_account();
    let t22_acc = token2022::keyed_account();

    // Bench 1: owner=spl-token, nested=spl-token
    let (ix1, accs1) = recover_nested_case(
        spl_token_interface::id(),
        spl_token_interface::id(),
        &spl_token_acc,
        &t22_acc,
    );

    // Bench 2: owner=token-2022, nested=token-2022
    let (ix2, accs2) = recover_nested_case(
        spl_token_2022_interface::id(),
        spl_token_2022_interface::id(),
        &spl_token_acc,
        &t22_acc,
    );

    // Bench 3: owner=spl-token, nested=token-2022
    let (ix3, accs3) = recover_nested_case(
        spl_token_interface::id(),
        spl_token_2022_interface::id(),
        &spl_token_acc,
        &t22_acc,
    );

    // Bench 4: owner=token-2022, nested=spl-token
    let (ix4, accs4) = recover_nested_case(
        spl_token_2022_interface::id(),
        spl_token_interface::id(),
        &spl_token_acc,
        &t22_acc,
    );

    MolluskComputeUnitBencher::new(mollusk)
        .bench((
            "recover_nested (owner=spl-token, nested=spl-token)",
            &ix1,
            &accs1,
        ))
        .bench((
            "recover_nested (owner=token-2022, nested=token-2022)",
            &ix2,
            &accs2,
        ))
        .bench((
            "recover_nested (owner=spl-token, nested=token-2022)",
            &ix3,
            &accs3,
        ))
        .bench((
            "recover_nested (owner=token-2022, nested=spl-token)",
            &ix4,
            &accs4,
        ))
        .must_pass(true)
        .out_dir("./benches")
        .execute();
}
