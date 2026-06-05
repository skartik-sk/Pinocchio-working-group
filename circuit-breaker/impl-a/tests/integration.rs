use litesvm::LiteSVM;
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_rent::Rent;
use solana_signer::Signer;
use solana_system_interface::instruction as sys_ix;
use solana_transaction::Transaction;

static PROGRAM_SO: &[u8] = include_bytes!("../target/deploy/escrow_circuit_breaker.so");

/// SPL Token program ID.
fn token_id() -> Pubkey {
    Pubkey::try_from("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap()
}

/// System program ID.
fn system_id() -> Pubkey {
    Pubkey::try_from("11111111111111111111111111111111").unwrap()
}

// ---------------------------------------------------------------------------
// SPL Token instruction builders (manual, to avoid spl-token crate version
// conflicts with litesvm's v3 solana-* deps)
// ---------------------------------------------------------------------------

/// Build an `InitializeMint2` instruction (disc 20 – no rent sysvar needed).
/// Data layout: [20, decimals(1), mint_authority(32), COption::None(33)]
fn token_init_mint_ix(mint: &Pubkey, authority: &Pubkey, decimals: u8) -> Instruction {
    let mut data = vec![20u8]; // InitializeMint2 disc
    data.push(decimals); // 1 byte
    data.extend_from_slice(authority.as_ref()); // 32 bytes
    data.extend_from_slice(&[0u8; 33]); // COption::None (1 byte tag + 32 bytes zeroed)
    // Total: 1 + 1 + 32 + 33 = 67 bytes
    Instruction {
        program_id: token_id(),
        accounts: vec![AccountMeta::new(*mint, false)],
        data,
    }
}

/// Build an `InitializeAccount3` instruction (disc 18 – no rent sysvar needed).
/// Data layout: [18, owner(32)].  Accounts: [account(writable), mint(readonly)].
fn token_init_account_ix(account: &Pubkey, mint: &Pubkey, owner: &Pubkey) -> Instruction {
    let mut data = vec![18u8]; // InitializeAccount3 disc
    data.extend_from_slice(owner.as_ref()); // 32 bytes
    // Total: 1 + 32 = 33 bytes
    Instruction {
        program_id: token_id(),
        accounts: vec![
            AccountMeta::new(*account, false),
            AccountMeta::new_readonly(*mint, false),
        ],
        data,
    }
}

/// Build a `MintTo` instruction (disc 7).
fn token_mint_to_ix(mint: &Pubkey, dest: &Pubkey, authority: &Pubkey, amount: u64) -> Instruction {
    let mut data = vec![7u8];
    data.extend_from_slice(&amount.to_le_bytes()); // 8 bytes
    Instruction {
        program_id: token_id(),
        accounts: vec![
            AccountMeta::new(*mint, false),                // mint (writable – supply changes)
            AccountMeta::new(*dest, false),                // destination
            AccountMeta::new_readonly(*authority, true),   // authority (signer)
        ],
        data,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup_svm() -> (LiteSVM, Pubkey) {
    let mut svm = LiteSVM::new();
    let program_id = Pubkey::new_unique();
    let _ = svm.add_program(program_id, PROGRAM_SO);
    (svm, program_id)
}

fn create_mint(svm: &mut LiteSVM, payer: &Keypair) -> Pubkey {
    let mint_kp = Keypair::new();
    let rent = Rent::default();
    let space: usize = 82; // Mint::LEN
    let lamports = rent.minimum_balance(space);

    let create_ix = sys_ix::create_account(
        &payer.pubkey(),
        &mint_kp.pubkey(),
        lamports,
        space as u64,
        &token_id(),
    );
    let init_ix = token_init_mint_ix(&mint_kp.pubkey(), &payer.pubkey(), 9);

    let tx = Transaction::new_signed_with_payer(
        &[create_ix, init_ix],
        Some(&payer.pubkey()),
        &[payer, &mint_kp],
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx).unwrap();

    mint_kp.pubkey()
}

fn create_token_account(
    svm: &mut LiteSVM,
    payer: &Keypair,
    owner: &Pubkey,
    mint: &Pubkey,
) -> Pubkey {
    let acc_kp = Keypair::new();
    let rent = Rent::default();
    let space: usize = 165; // spl_token::state::Account::LEN
    let lamports = rent.minimum_balance(space);

    let create_ix = sys_ix::create_account(
        &payer.pubkey(),
        &acc_kp.pubkey(),
        lamports,
        space as u64,
        &token_id(),
    );
    let init_ix = token_init_account_ix(&acc_kp.pubkey(), mint, owner);

    let tx = Transaction::new_signed_with_payer(
        &[create_ix, init_ix],
        Some(&payer.pubkey()),
        &[payer, &acc_kp],
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx).unwrap();

    acc_kp.pubkey()
}

fn mint_tokens(svm: &mut LiteSVM, payer: &Keypair, mint: &Pubkey, dest: &Pubkey, amount: u64) {
    let ix = token_mint_to_ix(mint, dest, &payer.pubkey(), amount);
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[payer],
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx).unwrap();
}

fn get_token_balance(svm: &LiteSVM, account: &Pubkey) -> u64 {
    let addr: Address = (*account).into();
    let acc = svm.get_account(&addr).unwrap();
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&acc.data[64..72]);
    u64::from_le_bytes(buf)
}

fn airdrop(svm: &mut LiteSVM, pubkey: &Pubkey, lamports: u64) {
    let addr: Address = (*pubkey).into();
    svm.airdrop(&addr, lamports).unwrap();
}

fn get_account_data(svm: &LiteSVM, pubkey: &Pubkey) -> Vec<u8> {
    let addr: Address = (*pubkey).into();
    svm.get_account(&addr).unwrap().data
}

/// Check that a failed transaction's logs contain a specific custom error code.
fn assert_error(result: std::result::Result<litesvm::types::TransactionMetadata, litesvm::types::FailedTransactionMetadata>, expected_code: u64) {
    let err = result.unwrap_err();
    let needle = format!("custom program error: 0x{:x}", expected_code);
    let found = err.meta.logs.iter().any(|l| l.contains(&needle));
    assert!(found, "Expected error {} but logs were:\n{}", expected_code, err.meta.logs.join("\n"));
}

/// Initialize a circuit-breaker. Returns the CB PDA.
fn init_cb(
    svm: &mut LiteSVM,
    program_id: &Pubkey,
    authority: &Keypair,
    window_sec: u64,
    threshold_type: u8,
    threshold: u64,
) -> Pubkey {
    let cb_pda = Pubkey::find_program_address(
        &[b"circuit-breaker", authority.pubkey().as_ref()],
        program_id,
    )
    .0;

    let mut data = vec![3u8]; // INIT_DISC
    data.extend_from_slice(&window_sec.to_le_bytes());
    data.push(threshold_type);
    data.extend_from_slice(&threshold.to_le_bytes());

    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(authority.pubkey(), true), // writable – pays for PDA creation
            AccountMeta::new(cb_pda, false),
            AccountMeta::new_readonly(system_id(), false),
        ],
        data,
    };

    svm.send_transaction(Transaction::new_signed_with_payer(
        &[ix],
        Some(&authority.pubkey()),
        &[authority],
        svm.latest_blockhash(),
    ))
    .unwrap();

    cb_pda
}

/// Create a vault token-account (for `mint`) owned by `owner`.
fn create_vault(
    svm: &mut LiteSVM,
    payer: &Keypair,
    mint: &Pubkey,
    owner: &Pubkey,
) -> (Keypair, Pubkey) {
    let vault_kp = Keypair::new();
    let rent = Rent::default();
    let space: usize = 165;
    let lamports = rent.minimum_balance(space);

    let create_ix = sys_ix::create_account(
        &payer.pubkey(),
        &vault_kp.pubkey(),
        lamports,
        space as u64,
        &token_id(),
    );
    let init_ix = token_init_account_ix(&vault_kp.pubkey(), mint, owner);

    let tx = Transaction::new_signed_with_payer(
        &[create_ix, init_ix],
        Some(&payer.pubkey()),
        &[payer, &vault_kp],
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx).unwrap();

    let vault_pubkey = vault_kp.pubkey();
    (vault_kp, vault_pubkey)
}

/// Build the `make` instruction.
fn build_make_ix(
    program_id: &Pubkey,
    maker: &Pubkey,
    mint_a: &Pubkey,
    mint_b: &Pubkey,
    maker_ata_a: &Pubkey,
    vault: &Pubkey,
    escrow_pda: &Pubkey,
    cb_pda: &Pubkey,
    amount: u64,
) -> Instruction {
    let mut data = vec![0u8]; // make DISC
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&i64::MAX.to_le_bytes()); // expiry = never

    Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(*maker, true),
            AccountMeta::new_readonly(*mint_a, false),
            AccountMeta::new_readonly(*mint_b, false),
            AccountMeta::new(*maker_ata_a, false),
            AccountMeta::new(*vault, false),
            AccountMeta::new(*escrow_pda, false),
            AccountMeta::new(*cb_pda, false),
            AccountMeta::new_readonly(system_id(), false),
            AccountMeta::new_readonly(token_id(), false),
        ],
        data,
    }
}

// ---------------------------------------------------------------------------
// Tests – circuit-breaker only
// ---------------------------------------------------------------------------

#[test]
fn test_init_circuit_breaker() {
    let (mut svm, program_id) = setup_svm();
    let authority = Keypair::new();
    airdrop(&mut svm, &authority.pubkey(), 10_000_000_000);

    let cb_pda = Pubkey::find_program_address(
        &[b"circuit-breaker", authority.pubkey().as_ref()],
        &program_id,
    )
    .0;

    let mut ix_data = vec![3u8];
    ix_data.extend_from_slice(&60u64.to_le_bytes());
    ix_data.push(1u8);
    ix_data.extend_from_slice(&1_000_000u64.to_le_bytes());

    let ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(authority.pubkey(), true), // writable – pays for PDA creation
            AccountMeta::new(cb_pda, false),
            AccountMeta::new_readonly(system_id(), false),
        ],
        data: ix_data,
    };

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[ix],
        Some(&authority.pubkey()),
        &[&authority],
        svm.latest_blockhash(),
    ));
    assert!(result.is_ok(), "CB init failed: {:?}", result.err());

    let cb_data = get_account_data(&svm, &cb_pda);
    assert_eq!(cb_data[32], 0, "Should start unpaused");
}

#[test]
fn test_pause_and_unpause() {
    let (mut svm, program_id) = setup_svm();
    let authority = Keypair::new();
    airdrop(&mut svm, &authority.pubkey(), 10_000_000_000);

    let cb_pda = Pubkey::find_program_address(
        &[b"circuit-breaker", authority.pubkey().as_ref()],
        &program_id,
    )
    .0;

    // Init CB
    let mut init_data = vec![3u8];
    init_data.extend_from_slice(&60u64.to_le_bytes());
    init_data.push(1u8);
    init_data.extend_from_slice(&1_000_000u64.to_le_bytes());

    svm.send_transaction(Transaction::new_signed_with_payer(
        &[Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(authority.pubkey(), true), // writable – pays for PDA creation
                AccountMeta::new(cb_pda, false),
                AccountMeta::new_readonly(system_id(), false),
            ],
            data: init_data,
        }],
        Some(&authority.pubkey()),
        &[&authority],
        svm.latest_blockhash(),
    ))
    .unwrap();

    // Pause
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new_readonly(authority.pubkey(), true),
                AccountMeta::new(cb_pda, false),
            ],
            data: vec![5u8, 1u8],
        }],
        Some(&authority.pubkey()),
        &[&authority],
        svm.latest_blockhash(),
    ))
    .unwrap();

    let cb_data = get_account_data(&svm, &cb_pda);
    assert_eq!(cb_data[32], 1, "Should be paused");

    // Unpause
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new_readonly(authority.pubkey(), true),
                AccountMeta::new(cb_pda, false),
            ],
            data: vec![5u8, 0u8],
        }],
        Some(&authority.pubkey()),
        &[&authority],
        svm.latest_blockhash(),
    ))
    .unwrap();

    let cb_data = get_account_data(&svm, &cb_pda);
    assert_eq!(cb_data[32], 0, "Should be unpaused");
}

#[test]
fn test_update_cb_params() {
    let (mut svm, program_id) = setup_svm();
    let authority = Keypair::new();
    airdrop(&mut svm, &authority.pubkey(), 10_000_000_000);

    let cb_pda = Pubkey::find_program_address(
        &[b"circuit-breaker", authority.pubkey().as_ref()],
        &program_id,
    )
    .0;

    // Init with window=60, threshold=1M
    let mut init_data = vec![3u8];
    init_data.extend_from_slice(&60u64.to_le_bytes());
    init_data.push(1u8);
    init_data.extend_from_slice(&1_000_000u64.to_le_bytes());

    svm.send_transaction(Transaction::new_signed_with_payer(
        &[Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(authority.pubkey(), true), // writable – pays for PDA creation
                AccountMeta::new(cb_pda, false),
                AccountMeta::new_readonly(system_id(), false),
            ],
            data: init_data,
        }],
        Some(&authority.pubkey()),
        &[&authority],
        svm.latest_blockhash(),
    ))
    .unwrap();

    // Update to window=120, threshold=5M
    let mut update_data = vec![4u8];       // disc
    update_data.extend_from_slice(&120u64.to_le_bytes()); // window_sec (bytes 1-8)
    update_data.push(1u8);                 // threshold_type (byte 9)
    update_data.extend_from_slice(&5_000_000u64.to_le_bytes()); // threshold (bytes 10-17)

    svm.send_transaction(Transaction::new_signed_with_payer(
        &[Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new_readonly(authority.pubkey(), true),
                AccountMeta::new(cb_pda, false),
            ],
            data: update_data,
        }],
        Some(&authority.pubkey()),
        &[&authority],
        svm.latest_blockhash(),
    ))
    .unwrap();

    let cb_data = get_account_data(&svm, &cb_pda);
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&cb_data[33..41]);
    assert_eq!(u64::from_le_bytes(buf), 120);
    buf.copy_from_slice(&cb_data[42..50]);
    assert_eq!(u64::from_le_bytes(buf), 5_000_000);
}

// ---------------------------------------------------------------------------
// Tests – escrow make / take / refund
// ---------------------------------------------------------------------------

#[test]
fn test_make_escrow_success() {
    let (mut svm, program_id) = setup_svm();

    let maker = Keypair::new();
    airdrop(&mut svm, &maker.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &maker);
    let mint_b = create_mint(&mut svm, &maker);
    let maker_ata_a = create_token_account(&mut svm, &maker, &maker.pubkey(), &mint_a);
    mint_tokens(&mut svm, &maker, &mint_a, &maker_ata_a, 10_000_000_000);

    let escrow_pda = Pubkey::find_program_address(
        &[b"escrow", maker.pubkey().as_ref()],
        &program_id,
    )
    .0;

    let cb_pda = init_cb(&mut svm, &program_id, &maker, 60, 1, u64::MAX);

    // Vault: token account for mint_a, owned by escrow PDA so take/refund can sign.
    let (_vault_kp, vault_pubkey) = create_vault(&mut svm, &maker, &mint_a, &escrow_pda);

    let amount: u64 = 1_000_000_000;
    let make_ix = build_make_ix(
        &program_id,
        &maker.pubkey(),
        &mint_a,
        &mint_b,
        &maker_ata_a,
        &vault_pubkey,
        &escrow_pda,
        &cb_pda,
        amount,
    );

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[make_ix],
        Some(&maker.pubkey()),
        &[&maker],
        svm.latest_blockhash(),
    ));
    assert!(result.is_ok(), "Make escrow failed: {:?}", result.err());

    // Vault received the tokens
    assert_eq!(get_token_balance(&svm, &vault_pubkey), amount);

    // Maker balance decreased
    assert_eq!(
        get_token_balance(&svm, &maker_ata_a),
        10_000_000_000 - amount
    );

    // Escrow account was created
    let escrow_data = get_account_data(&svm, &escrow_pda);
    assert_eq!(escrow_data.len(), 113); // Escrow::LEN
}

#[test]
fn test_take_escrow_full_flow() {
    let (mut svm, program_id) = setup_svm();

    let maker = Keypair::new();
    let taker = Keypair::new();
    airdrop(&mut svm, &maker.pubkey(), 50_000_000_000);
    airdrop(&mut svm, &taker.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &maker);
    let mint_b = create_mint(&mut svm, &maker);

    let maker_ata_a = create_token_account(&mut svm, &maker, &maker.pubkey(), &mint_a);
    let maker_ata_b = create_token_account(&mut svm, &maker, &maker.pubkey(), &mint_b);
    let taker_ata_a = create_token_account(&mut svm, &taker, &taker.pubkey(), &mint_a);
    let taker_ata_b = create_token_account(&mut svm, &taker, &taker.pubkey(), &mint_b);

    mint_tokens(&mut svm, &maker, &mint_a, &maker_ata_a, 10_000_000_000);
    mint_tokens(&mut svm, &maker, &mint_b, &taker_ata_b, 10_000_000_000);

    let escrow_pda = Pubkey::find_program_address(
        &[b"escrow", maker.pubkey().as_ref()],
        &program_id,
    )
    .0;

    let cb_pda = init_cb(&mut svm, &program_id, &maker, 60, 1, u64::MAX);

    // Vault: mint_a token account owned by escrow PDA
    let (_vault_kp, vault_pubkey) = create_vault(&mut svm, &maker, &mint_a, &escrow_pda);

    // ---- Make ----
    let amount: u64 = 1_000_000_000;
    let make_ix = build_make_ix(
        &program_id,
        &maker.pubkey(),
        &mint_a,
        &mint_b,
        &maker_ata_a,
        &vault_pubkey,
        &escrow_pda,
        &cb_pda,
        amount,
    );
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[make_ix],
        Some(&maker.pubkey()),
        &[&maker],
        svm.latest_blockhash(),
    ))
    .unwrap();

    // ---- Take ----
    // NOTE: The program's take instruction destructures the 5th account as
    // `maker_ata_a` but uses it as the destination for mint_b tokens.
    // We pass `maker_ata_b` so the SPL token Transfer succeeds (same mint).
    let take_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(taker.pubkey(), true),     // taker (signer)
            AccountMeta::new(maker.pubkey(), false),              // maker
            AccountMeta::new_readonly(mint_a, false),             // mint_a
            AccountMeta::new_readonly(mint_b, false),             // mint_b
            AccountMeta::new(maker_ata_b, false),                 // dest for mint_b
            AccountMeta::new(taker_ata_a, false),                 // taker_ata_a
            AccountMeta::new(taker_ata_b, false),                 // taker_ata_b
            AccountMeta::new(vault_pubkey, false),                // vault
            AccountMeta::new(escrow_pda, false),                  // escrow
            AccountMeta::new(cb_pda, false),                      // cb_pda (writable – window update)
            AccountMeta::new_readonly(token_id(), false),         // token_program
        ],
        data: vec![1u8], // take DISC
    };

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[take_ix],
        Some(&taker.pubkey()),
        &[&taker],
        svm.latest_blockhash(),
    ));
    assert!(result.is_ok(), "Take escrow failed: {:?}", result.err());

    // Taker received mint_a tokens from vault
    assert_eq!(
        get_token_balance(&svm, &taker_ata_a),
        amount,
        "Taker should have received mint_a tokens"
    );

    // Maker received mint_b tokens from taker
    assert_eq!(
        get_token_balance(&svm, &maker_ata_b),
        amount,
        "Maker should have received mint_b tokens"
    );

    // Taker's mint_b balance decreased
    assert_eq!(
        get_token_balance(&svm, &taker_ata_b),
        10_000_000_000 - amount,
        "Taker's mint_b should have decreased"
    );

    // Vault closed after take (account no longer exists)
    let vault_addr: Address = vault_pubkey.into();
    assert!(
        svm.get_account(&vault_addr).is_none_or(|a| a.lamports == 0),
        "Vault should be closed after take"
    );

    // Escrow PDA closed (lamports drained to maker)
    let escrow_addr: Address = escrow_pda.into();
    assert!(
        svm.get_account(&escrow_addr).is_none_or(|a| a.lamports == 0),
        "Escrow should be closed after take"
    );
}

#[test]
fn test_refund_escrow() {
    let (mut svm, program_id) = setup_svm();

    let maker = Keypair::new();
    airdrop(&mut svm, &maker.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &maker);
    let mint_b = create_mint(&mut svm, &maker);
    let maker_ata_a = create_token_account(&mut svm, &maker, &maker.pubkey(), &mint_a);
    mint_tokens(&mut svm, &maker, &mint_a, &maker_ata_a, 10_000_000_000);

    let escrow_pda = Pubkey::find_program_address(
        &[b"escrow", maker.pubkey().as_ref()],
        &program_id,
    )
    .0;

    let cb_pda = init_cb(&mut svm, &program_id, &maker, 60, 1, u64::MAX);

    let (_vault_kp, vault_pubkey) = create_vault(&mut svm, &maker, &mint_a, &escrow_pda);

    // ---- Make ----
    let amount: u64 = 1_000_000_000;
    let make_ix = build_make_ix(
        &program_id,
        &maker.pubkey(),
        &mint_a,
        &mint_b,
        &maker_ata_a,
        &vault_pubkey,
        &escrow_pda,
        &cb_pda,
        amount,
    );
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[make_ix],
        Some(&maker.pubkey()),
        &[&maker],
        svm.latest_blockhash(),
    ))
    .unwrap();

    assert_eq!(get_token_balance(&svm, &maker_ata_a), 10_000_000_000 - amount);
    assert_eq!(get_token_balance(&svm, &vault_pubkey), amount);

    // ---- Refund ----
    let refund_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(maker.pubkey(), true),                // maker (signer)
            AccountMeta::new(maker_ata_a, false),                  // maker_ata_a
            AccountMeta::new(vault_pubkey, false),                 // vault
            AccountMeta::new(escrow_pda, false),                   // escrow
            AccountMeta::new_readonly(token_id(), false),          // token_program
        ],
        data: vec![2u8], // refund DISC
    };

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[refund_ix],
        Some(&maker.pubkey()),
        &[&maker],
        svm.latest_blockhash(),
    ));
    assert!(result.is_ok(), "Refund failed: {:?}", result.err());

    // Tokens returned to maker
    assert_eq!(
        get_token_balance(&svm, &maker_ata_a),
        10_000_000_000,
        "Tokens should be returned to maker"
    );

    // Vault closed (account no longer exists)
    let vault_addr: Address = vault_pubkey.into();
    assert!(
        svm.get_account(&vault_addr).is_none_or(|a| a.lamports == 0),
        "Vault should be closed after refund"
    );
}

// ---------------------------------------------------------------------------
// Tests – circuit-breaker enforcement
// ---------------------------------------------------------------------------

#[test]
fn test_make_blocked_when_paused() {
    let (mut svm, program_id) = setup_svm();

    let maker = Keypair::new();
    airdrop(&mut svm, &maker.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &maker);
    let mint_b = create_mint(&mut svm, &maker);
    let maker_ata_a = create_token_account(&mut svm, &maker, &maker.pubkey(), &mint_a);
    mint_tokens(&mut svm, &maker, &mint_a, &maker_ata_a, 10_000_000_000);

    let escrow_pda = Pubkey::find_program_address(
        &[b"escrow", maker.pubkey().as_ref()],
        &program_id,
    )
    .0;

    let cb_pda = init_cb(&mut svm, &program_id, &maker, 60, 1, u64::MAX);

    // Pause the circuit breaker
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new_readonly(maker.pubkey(), true),
                AccountMeta::new(cb_pda, false),
            ],
            data: vec![5u8, 1u8],
        }],
        Some(&maker.pubkey()),
        &[&maker],
        svm.latest_blockhash(),
    ))
    .unwrap();

    let (_vault_kp, vault_pubkey) = create_vault(&mut svm, &maker, &mint_a, &escrow_pda);

    let amount: u64 = 1_000_000_000;
    let make_ix = build_make_ix(
        &program_id,
        &maker.pubkey(),
        &mint_a,
        &mint_b,
        &maker_ata_a,
        &vault_pubkey,
        &escrow_pda,
        &cb_pda,
        amount,
    );

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[make_ix],
        Some(&maker.pubkey()),
        &[&maker],
        svm.latest_blockhash(),
    ));

    assert_error(result, 6001); // ERR_PAUSED
}

#[test]
fn test_take_blocked_when_paused() {
    let (mut svm, program_id) = setup_svm();

    let maker = Keypair::new();
    let taker = Keypair::new();
    airdrop(&mut svm, &maker.pubkey(), 50_000_000_000);
    airdrop(&mut svm, &taker.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &maker);
    let mint_b = create_mint(&mut svm, &maker);

    let maker_ata_a = create_token_account(&mut svm, &maker, &maker.pubkey(), &mint_a);
    let maker_ata_b = create_token_account(&mut svm, &maker, &maker.pubkey(), &mint_b);
    let taker_ata_a = create_token_account(&mut svm, &taker, &taker.pubkey(), &mint_a);
    let taker_ata_b = create_token_account(&mut svm, &taker, &taker.pubkey(), &mint_b);

    mint_tokens(&mut svm, &maker, &mint_a, &maker_ata_a, 10_000_000_000);
    mint_tokens(&mut svm, &maker, &mint_b, &taker_ata_b, 10_000_000_000);

    let escrow_pda = Pubkey::find_program_address(
        &[b"escrow", maker.pubkey().as_ref()],
        &program_id,
    )
    .0;

    let cb_pda = init_cb(&mut svm, &program_id, &maker, 60, 1, u64::MAX);

    let (_vault_kp, vault_pubkey) = create_vault(&mut svm, &maker, &mint_a, &escrow_pda);

    // Make (succeeds – CB not paused)
    let amount: u64 = 1_000_000_000;
    let make_ix = build_make_ix(
        &program_id,
        &maker.pubkey(),
        &mint_a,
        &mint_b,
        &maker_ata_a,
        &vault_pubkey,
        &escrow_pda,
        &cb_pda,
        amount,
    );
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[make_ix],
        Some(&maker.pubkey()),
        &[&maker],
        svm.latest_blockhash(),
    ))
    .unwrap();

    // Pause CB
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new_readonly(maker.pubkey(), true),
                AccountMeta::new(cb_pda, false),
            ],
            data: vec![5u8, 1u8],
        }],
        Some(&maker.pubkey()),
        &[&maker],
        svm.latest_blockhash(),
    ))
    .unwrap();

    // Take (should fail – CB paused)
    let take_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(taker.pubkey(), true),
            AccountMeta::new(maker.pubkey(), false),
            AccountMeta::new_readonly(mint_a, false),
            AccountMeta::new_readonly(mint_b, false),
            AccountMeta::new(maker_ata_b, false),
            AccountMeta::new(taker_ata_a, false),
            AccountMeta::new(taker_ata_b, false),
            AccountMeta::new(vault_pubkey, false),
            AccountMeta::new(escrow_pda, false),
            AccountMeta::new(cb_pda, false),
            AccountMeta::new_readonly(token_id(), false),
        ],
        data: vec![1u8],
    };

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[take_ix],
        Some(&taker.pubkey()),
        &[&taker],
        svm.latest_blockhash(),
    ));

    assert_error(result, 6001); // ERR_PAUSED
}

#[test]
fn test_take_blocked_by_threshold() {
    let (mut svm, program_id) = setup_svm();

    let maker = Keypair::new();
    let taker = Keypair::new();
    airdrop(&mut svm, &maker.pubkey(), 50_000_000_000);
    airdrop(&mut svm, &taker.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &maker);
    let mint_b = create_mint(&mut svm, &maker);

    let maker_ata_a = create_token_account(&mut svm, &maker, &maker.pubkey(), &mint_a);
    let maker_ata_b = create_token_account(&mut svm, &maker, &maker.pubkey(), &mint_b);
    let taker_ata_a = create_token_account(&mut svm, &taker, &taker.pubkey(), &mint_a);
    let taker_ata_b = create_token_account(&mut svm, &taker, &taker.pubkey(), &mint_b);

    mint_tokens(&mut svm, &maker, &mint_a, &maker_ata_a, 10_000_000_000);
    mint_tokens(&mut svm, &maker, &mint_b, &taker_ata_b, 10_000_000_000);

    let escrow_pda = Pubkey::find_program_address(
        &[b"escrow", maker.pubkey().as_ref()],
        &program_id,
    )
    .0;

    // CB with very low threshold – amount (1B) >> threshold (100)
    let cb_pda = init_cb(&mut svm, &program_id, &maker, 60, 1, 100);

    let (_vault_kp, vault_pubkey) = create_vault(&mut svm, &maker, &mint_a, &escrow_pda);

    // Make succeeds – make only checks paused, not threshold
    let amount: u64 = 1_000_000_000;
    let make_ix = build_make_ix(
        &program_id,
        &maker.pubkey(),
        &mint_a,
        &mint_b,
        &maker_ata_a,
        &vault_pubkey,
        &escrow_pda,
        &cb_pda,
        amount,
    );
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[make_ix],
        Some(&maker.pubkey()),
        &[&maker],
        svm.latest_blockhash(),
    ))
    .unwrap();

    // Take should fail – amount exceeds threshold
    let take_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(taker.pubkey(), true),
            AccountMeta::new(maker.pubkey(), false),
            AccountMeta::new_readonly(mint_a, false),
            AccountMeta::new_readonly(mint_b, false),
            AccountMeta::new(maker_ata_b, false),
            AccountMeta::new(taker_ata_a, false),
            AccountMeta::new(taker_ata_b, false),
            AccountMeta::new(vault_pubkey, false),
            AccountMeta::new(escrow_pda, false),
            AccountMeta::new(cb_pda, false),
            AccountMeta::new_readonly(token_id(), false),
        ],
        data: vec![1u8],
    };

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[take_ix],
        Some(&taker.pubkey()),
        &[&taker],
        svm.latest_blockhash(),
    ));

    assert_error(result, 6002); // ERR_CB_TRIGGERED
}
