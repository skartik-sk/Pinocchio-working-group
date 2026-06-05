use litesvm::LiteSVM;
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_rent::Rent;
use solana_signer::Signer;
use solana_system_interface::instruction as sys_ix;
use solana_transaction::Transaction;

static PROGRAM_SO: &[u8] = include_bytes!("../target/deploy/stable_swap_circuit_breaker.so");

fn token_id() -> Pubkey {
    Pubkey::try_from("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap()
}

fn system_id() -> Pubkey {
    Pubkey::try_from("11111111111111111111111111111111").unwrap()
}

// ---------------------------------------------------------------------------
// SPL Token instruction builders
// ---------------------------------------------------------------------------

fn token_init_mint_ix(mint: &Pubkey, authority: &Pubkey, decimals: u8) -> Instruction {
    let mut data = vec![20u8];
    data.push(decimals);
    data.extend_from_slice(authority.as_ref());
    data.extend_from_slice(&[0u8; 33]); // COption::None
    Instruction { program_id: token_id(), accounts: vec![AccountMeta::new(*mint, false)], data }
}

fn token_init_account_ix(account: &Pubkey, mint: &Pubkey, owner: &Pubkey) -> Instruction {
    let mut data = vec![18u8];
    data.extend_from_slice(owner.as_ref());
    Instruction {
        program_id: token_id(),
        accounts: vec![AccountMeta::new(*account, false), AccountMeta::new_readonly(*mint, false)],
        data,
    }
}

fn token_mint_to_ix(mint: &Pubkey, dest: &Pubkey, authority: &Pubkey, amount: u64) -> Instruction {
    let mut data = vec![7u8];
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: token_id(),
        accounts: vec![
            AccountMeta::new(*mint, false),
            AccountMeta::new(*dest, false),
            AccountMeta::new_readonly(*authority, true),
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

fn airdrop(svm: &mut LiteSVM, pubkey: &Pubkey, lamports: u64) {
    let addr: Address = (*pubkey).into();
    svm.airdrop(&addr, lamports).unwrap();
}

fn create_mint(svm: &mut LiteSVM, payer: &Keypair) -> Pubkey {
    let mint_kp = Keypair::new();
    let rent = Rent::default();
    let space: usize = 82;
    let lamports = rent.minimum_balance(space);

    let create_ix = sys_ix::create_account(&payer.pubkey(), &mint_kp.pubkey(), lamports, space as u64, &token_id());
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

fn create_token_account(svm: &mut LiteSVM, payer: &Keypair, owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    let acc_kp = Keypair::new();
    let rent = Rent::default();
    let space: usize = 165;
    let lamports = rent.minimum_balance(space);

    let create_ix = sys_ix::create_account(&payer.pubkey(), &acc_kp.pubkey(), lamports, space as u64, &token_id());
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
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[payer], svm.latest_blockhash());
    svm.send_transaction(tx).unwrap();
}

fn get_token_balance(svm: &LiteSVM, account: &Pubkey) -> u64 {
    let addr: Address = (*account).into();
    let acc = svm.get_account(&addr).unwrap();
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&acc.data[64..72]);
    u64::from_le_bytes(buf)
}

fn get_account_data(svm: &LiteSVM, pubkey: &Pubkey) -> Vec<u8> {
    let addr: Address = (*pubkey).into();
    svm.get_account(&addr).unwrap().data
}

fn assert_error(result: Result<litesvm::types::TransactionMetadata, litesvm::types::FailedTransactionMetadata>, expected_code: u64) {
    let err = result.unwrap_err();
    let needle = format!("custom program error: 0x{:x}", expected_code);
    let found = err.meta.logs.iter().any(|l| l.contains(&needle));
    assert!(found, "Expected error {} but logs were:\n{}", expected_code, err.meta.logs.join("\n"));
}

/// Derive the pool PDA from mint_a and mint_b.
fn derive_pool_pda(program_id: &Pubkey, mint_a: &Pubkey, mint_b: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"stable-swap-pool", mint_a.as_ref(), mint_b.as_ref()],
        program_id,
    )
}

/// Initialize a pool. Returns (pool_pda, bump).
fn init_pool(svm: &mut LiteSVM, program_id: &Pubkey, authority: &Keypair, mint_a: &Pubkey, mint_b: &Pubkey, amp: u64) -> (Pubkey, u8) {
    let (pool_pda, _bump) = derive_pool_pda(program_id, mint_a, mint_b);

    let mut data = vec![0u8]; // INIT_DISC
    data.extend_from_slice(&amp.to_le_bytes());

    let ix = Instruction {
        program_id: *program_id,
        accounts: vec![
            AccountMeta::new(authority.pubkey(), true),       // authority (writable – pays)
            AccountMeta::new_readonly(*mint_a, false),         // mint_a
            AccountMeta::new_readonly(*mint_b, false),         // mint_b
            AccountMeta::new(pool_pda, false),                 // pool_state
            AccountMeta::new_readonly(system_id(), false),     // system_program
            AccountMeta::new_readonly(token_id(), false),      // token_program
        ],
        data,
    };

    svm.send_transaction(Transaction::new_signed_with_payer(
        &[ix],
        Some(&authority.pubkey()),
        &[authority],
        svm.latest_blockhash(),
    )).unwrap();

    (pool_pda, _bump)
}

/// Create reserve token accounts and LP mint owned by pool PDA.
struct PoolAccounts {
    reserve_a: Pubkey,
    reserve_b: Pubkey,
    lp_mint: Pubkey,
}

fn create_pool_accounts(svm: &mut LiteSVM, payer: &Keypair, mint_a: &Pubkey, mint_b: &Pubkey, pool_pda: &Pubkey) -> PoolAccounts {
    let reserve_a = create_token_account(svm, payer, pool_pda, mint_a);
    let reserve_b = create_token_account(svm, payer, pool_pda, mint_b);
    let lp_mint = create_mint_for(svm, payer, pool_pda);
    PoolAccounts { reserve_a, reserve_b, lp_mint }
}

/// Create a mint with the given owner (for LP mint owned by pool PDA).
fn create_mint_for(svm: &mut LiteSVM, payer: &Keypair, owner: &Pubkey) -> Pubkey {
    let mint_kp = Keypair::new();
    let rent = Rent::default();
    let space: usize = 82;
    let lamports = rent.minimum_balance(space);

    let create_ix = sys_ix::create_account(&payer.pubkey(), &mint_kp.pubkey(), lamports, space as u64, &token_id());

    // InitializeMint2 with owner as mint authority and freeze authority
    let mut data = vec![20u8];
    data.push(9);
    data.extend_from_slice(owner.as_ref()); // mint_authority
    data.extend_from_slice(&[0u8; 33]);     // COption::None for freeze
    let init_ix = Instruction { program_id: token_id(), accounts: vec![AccountMeta::new(mint_kp.pubkey(), false)], data };

    let tx = Transaction::new_signed_with_payer(
        &[create_ix, init_ix],
        Some(&payer.pubkey()),
        &[payer, &mint_kp],
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx).unwrap();
    mint_kp.pubkey()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_init_pool() {
    let (mut svm, program_id) = setup_svm();
    let authority = Keypair::new();
    airdrop(&mut svm, &authority.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &authority);
    let mint_b = create_mint(&mut svm, &authority);

    let (pool_pda, _bump) = init_pool(&mut svm, &program_id, &authority, &mint_a, &mint_b, 100);

    let pool_data = get_account_data(&svm, &pool_pda);
    assert_eq!(pool_data.len(), 238, "PoolState::LEN should be 238");
    assert_eq!(pool_data[204], 0, "Should start unpaused");

    // Verify amp factor
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&pool_data[192..200]);
    assert_eq!(u64::from_le_bytes(buf), 100);

    // Verify default CB params
    buf.copy_from_slice(&pool_data[205..213]);
    assert_eq!(u64::from_le_bytes(buf), 60, "Default CB window should be 60");
    buf.copy_from_slice(&pool_data[213..221]);
    assert_eq!(u64::from_le_bytes(buf), 1_000_000_000, "Default CB threshold should be 1B");
}

#[test]
fn test_pause_and_unpause() {
    let (mut svm, program_id) = setup_svm();
    let authority = Keypair::new();
    airdrop(&mut svm, &authority.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &authority);
    let mint_b = create_mint(&mut svm, &authority);
    let (pool_pda, _) = init_pool(&mut svm, &program_id, &authority, &mint_a, &mint_b, 100);

    // Pause
    let pause_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(authority.pubkey(), true),
            AccountMeta::new(pool_pda, false),
        ],
        data: vec![3u8, 1u8],
    };
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[pause_ix], Some(&authority.pubkey()), &[&authority], svm.latest_blockhash(),
    )).unwrap();

    let pool_data = get_account_data(&svm, &pool_pda);
    assert_eq!(pool_data[204], 1, "Should be paused");

    // Unpause
    let unpause_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(authority.pubkey(), true),
            AccountMeta::new(pool_pda, false),
        ],
        data: vec![3u8, 0u8],
    };
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[unpause_ix], Some(&authority.pubkey()), &[&authority], svm.latest_blockhash(),
    )).unwrap();

    let pool_data = get_account_data(&svm, &pool_pda);
    assert_eq!(pool_data[204], 0, "Should be unpaused");
}

#[test]
fn test_update_cb_params() {
    let (mut svm, program_id) = setup_svm();
    let authority = Keypair::new();
    airdrop(&mut svm, &authority.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &authority);
    let mint_b = create_mint(&mut svm, &authority);
    let (pool_pda, _) = init_pool(&mut svm, &program_id, &authority, &mint_a, &mint_b, 100);

    // Update CB: window=120, threshold=5M
    let mut update_data = vec![4u8];
    update_data.extend_from_slice(&120u64.to_le_bytes());  // window_sec at offset 1
    update_data.extend_from_slice(&5_000_000u64.to_le_bytes()); // threshold at offset 9

    let update_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(authority.pubkey(), true),
            AccountMeta::new(pool_pda, false),
        ],
        data: update_data,
    };
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[update_ix], Some(&authority.pubkey()), &[&authority], svm.latest_blockhash(),
    )).unwrap();

    let pool_data = get_account_data(&svm, &pool_pda);
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&pool_data[205..213]);
    assert_eq!(u64::from_le_bytes(buf), 120, "CB window should be 120");
    buf.copy_from_slice(&pool_data[213..221]);
    assert_eq!(u64::from_le_bytes(buf), 5_000_000, "CB threshold should be 5M");
}

#[test]
fn test_add_liquidity() {
    let (mut svm, program_id) = setup_svm();
    let authority = Keypair::new();
    let user = Keypair::new();
    airdrop(&mut svm, &authority.pubkey(), 50_000_000_000);
    airdrop(&mut svm, &user.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &authority);
    let mint_b = create_mint(&mut svm, &authority);

    let (pool_pda, _) = init_pool(&mut svm, &program_id, &authority, &mint_a, &mint_b, 100);
    let pool_accs = create_pool_accounts(&mut svm, &authority, &mint_a, &mint_b, &pool_pda);

    // User token accounts
    let user_ta_a = create_token_account(&mut svm, &user, &user.pubkey(), &mint_a);
    let user_ta_b = create_token_account(&mut svm, &user, &user.pubkey(), &mint_b);
    let user_lp = create_token_account(&mut svm, &user, &user.pubkey(), &pool_accs.lp_mint);

    let amt_a: u64 = 1_000_000_000;
    let amt_b: u64 = 1_000_000_000;
    mint_tokens(&mut svm, &authority, &mint_a, &user_ta_a, amt_a);
    mint_tokens(&mut svm, &authority, &mint_b, &user_ta_b, amt_b);

    // Add liquidity
    let mut add_data = vec![2u8];
    add_data.extend_from_slice(&amt_a.to_le_bytes());
    add_data.extend_from_slice(&amt_b.to_le_bytes());

    let add_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(user.pubkey(), true),                // user
            AccountMeta::new(pool_pda, false),                    // pool_state
            AccountMeta::new(user_ta_a, false),                   // user_mint_a
            AccountMeta::new(user_ta_b, false),                   // user_mint_b
            AccountMeta::new(pool_accs.reserve_a, false),         // reserve_a
            AccountMeta::new(pool_accs.reserve_b, false),         // reserve_b
            AccountMeta::new(pool_accs.lp_mint, false),           // lp_mint
            AccountMeta::new(user_lp, false),                     // user_lp
            AccountMeta::new_readonly(token_id(), false),         // token_program
        ],
        data: add_data,
    };

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[add_ix], Some(&user.pubkey()), &[&user], svm.latest_blockhash(),
    ));
    assert!(result.is_ok(), "Add liquidity failed: {:?}", result.err());

    // Reserves received tokens
    assert_eq!(get_token_balance(&svm, &pool_accs.reserve_a), amt_a);
    assert_eq!(get_token_balance(&svm, &pool_accs.reserve_b), amt_b);

    // User got LP tokens
    let lp_balance = get_token_balance(&svm, &user_lp);
    assert!(lp_balance > 0, "User should have received LP tokens");
}

#[test]
fn test_swap() {
    let (mut svm, program_id) = setup_svm();
    let authority = Keypair::new();
    let user = Keypair::new();
    airdrop(&mut svm, &authority.pubkey(), 50_000_000_000);
    airdrop(&mut svm, &user.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &authority);
    let mint_b = create_mint(&mut svm, &authority);

    let (pool_pda, _) = init_pool(&mut svm, &program_id, &authority, &mint_a, &mint_b, 100);
    let pool_accs = create_pool_accounts(&mut svm, &authority, &mint_a, &mint_b, &pool_pda);

    let user_ta_a = create_token_account(&mut svm, &user, &user.pubkey(), &mint_a);
    let user_ta_b = create_token_account(&mut svm, &user, &user.pubkey(), &mint_b);
    let user_lp = create_token_account(&mut svm, &user, &user.pubkey(), &pool_accs.lp_mint);

    let amt_a: u64 = 10_000_000_000;
    let amt_b: u64 = 10_000_000_000;
    mint_tokens(&mut svm, &authority, &mint_a, &user_ta_a, amt_a * 2);
    mint_tokens(&mut svm, &authority, &mint_b, &user_ta_b, amt_b);

    // Add liquidity first
    let mut add_data = vec![2u8];
    add_data.extend_from_slice(&amt_a.to_le_bytes());
    add_data.extend_from_slice(&amt_b.to_le_bytes());

    let add_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(user.pubkey(), true),
            AccountMeta::new(pool_pda, false),
            AccountMeta::new(user_ta_a, false),
            AccountMeta::new(user_ta_b, false),
            AccountMeta::new(pool_accs.reserve_a, false),
            AccountMeta::new(pool_accs.reserve_b, false),
            AccountMeta::new(pool_accs.lp_mint, false),
            AccountMeta::new(user_lp, false),
            AccountMeta::new_readonly(token_id(), false),
        ],
        data: add_data,
    };
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[add_ix], Some(&user.pubkey()), &[&user], svm.latest_blockhash(),
    )).unwrap();

    // Now swap: user trades mint_a for mint_b
    let swap_amount: u64 = 100_000_000; // 0.1 tokens
    let mut swap_data = vec![1u8]; // SWAP_DISC
    swap_data.extend_from_slice(&swap_amount.to_le_bytes());
    swap_data.extend_from_slice(&0u64.to_le_bytes()); // min_out = 0

    let swap_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(user.pubkey(), true),                // user
            AccountMeta::new(pool_pda, false),                    // pool_state
            AccountMeta::new(user_ta_a, false),                   // user_source (paying mint_a)
            AccountMeta::new(user_ta_b, false),                   // user_dest (receiving mint_b)
            AccountMeta::new(pool_accs.reserve_a, false),         // reserve_in
            AccountMeta::new(pool_accs.reserve_b, false),         // reserve_out
            AccountMeta::new_readonly(token_id(), false),         // token_program
        ],
        data: swap_data,
    };

    let user_b_before = get_token_balance(&svm, &user_ta_b);

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[swap_ix], Some(&user.pubkey()), &[&user], svm.latest_blockhash(),
    ));
    assert!(result.is_ok(), "Swap failed: {:?}", result.err());

    let user_b_after = get_token_balance(&svm, &user_ta_b);
    assert!(user_b_after > user_b_before, "User should have received mint_b tokens from swap");
}

#[test]
fn test_swap_blocked_when_paused() {
    let (mut svm, program_id) = setup_svm();
    let authority = Keypair::new();
    let user = Keypair::new();
    airdrop(&mut svm, &authority.pubkey(), 50_000_000_000);
    airdrop(&mut svm, &user.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &authority);
    let mint_b = create_mint(&mut svm, &authority);

    let (pool_pda, _) = init_pool(&mut svm, &program_id, &authority, &mint_a, &mint_b, 100);
    let pool_accs = create_pool_accounts(&mut svm, &authority, &mint_a, &mint_b, &pool_pda);

    let user_ta_a = create_token_account(&mut svm, &user, &user.pubkey(), &mint_a);
    let user_ta_b = create_token_account(&mut svm, &user, &user.pubkey(), &mint_b);
    let user_lp = create_token_account(&mut svm, &user, &user.pubkey(), &pool_accs.lp_mint);

    mint_tokens(&mut svm, &authority, &mint_a, &user_ta_a, 20_000_000_000);
    mint_tokens(&mut svm, &authority, &mint_b, &user_ta_b, 10_000_000_000);

    // Add liquidity
    let mut add_data = vec![2u8];
    add_data.extend_from_slice(&10_000_000_000u64.to_le_bytes());
    add_data.extend_from_slice(&10_000_000_000u64.to_le_bytes());
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(user.pubkey(), true),
                AccountMeta::new(pool_pda, false),
                AccountMeta::new(user_ta_a, false),
                AccountMeta::new(user_ta_b, false),
                AccountMeta::new(pool_accs.reserve_a, false),
                AccountMeta::new(pool_accs.reserve_b, false),
                AccountMeta::new(pool_accs.lp_mint, false),
                AccountMeta::new(user_lp, false),
                AccountMeta::new_readonly(token_id(), false),
            ],
            data: add_data,
        }],
        Some(&user.pubkey()), &[&user], svm.latest_blockhash(),
    )).unwrap();

    // Pause
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new_readonly(authority.pubkey(), true),
                AccountMeta::new(pool_pda, false),
            ],
            data: vec![3u8, 1u8],
        }],
        Some(&authority.pubkey()), &[&authority], svm.latest_blockhash(),
    )).unwrap();

    // Swap should fail with ERR_PAUSED
    let mut swap_data = vec![1u8];
    swap_data.extend_from_slice(&100_000_000u64.to_le_bytes());
    swap_data.extend_from_slice(&0u64.to_le_bytes());

    let swap_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(user.pubkey(), true),
            AccountMeta::new(pool_pda, false),
            AccountMeta::new(user_ta_a, false),
            AccountMeta::new(user_ta_b, false),
            AccountMeta::new(pool_accs.reserve_a, false),
            AccountMeta::new(pool_accs.reserve_b, false),
            AccountMeta::new_readonly(token_id(), false),
        ],
        data: swap_data,
    };

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[swap_ix], Some(&user.pubkey()), &[&user], svm.latest_blockhash(),
    ));
    assert_error(result, 7000); // ERR_PAUSED
}

#[test]
fn test_swap_blocked_by_threshold() {
    let (mut svm, program_id) = setup_svm();
    let authority = Keypair::new();
    let user = Keypair::new();
    airdrop(&mut svm, &authority.pubkey(), 50_000_000_000);
    airdrop(&mut svm, &user.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &authority);
    let mint_b = create_mint(&mut svm, &authority);

    let (pool_pda, _) = init_pool(&mut svm, &program_id, &authority, &mint_a, &mint_b, 100);
    let pool_accs = create_pool_accounts(&mut svm, &authority, &mint_a, &mint_b, &pool_pda);

    // Set very low CB threshold
    let mut cb_data = vec![4u8];
    cb_data.extend_from_slice(&60u64.to_le_bytes());
    cb_data.extend_from_slice(&100u64.to_le_bytes()); // threshold = 100

    svm.send_transaction(Transaction::new_signed_with_payer(
        &[Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new_readonly(authority.pubkey(), true),
                AccountMeta::new(pool_pda, false),
            ],
            data: cb_data,
        }],
        Some(&authority.pubkey()), &[&authority], svm.latest_blockhash(),
    )).unwrap();

    let user_ta_a = create_token_account(&mut svm, &user, &user.pubkey(), &mint_a);
    let user_ta_b = create_token_account(&mut svm, &user, &user.pubkey(), &mint_b);
    let user_lp = create_token_account(&mut svm, &user, &user.pubkey(), &pool_accs.lp_mint);

    mint_tokens(&mut svm, &authority, &mint_a, &user_ta_a, 20_000_000_000);
    mint_tokens(&mut svm, &authority, &mint_b, &user_ta_b, 10_000_000_000);

    // Add liquidity
    let mut add_data = vec![2u8];
    add_data.extend_from_slice(&10_000_000_000u64.to_le_bytes());
    add_data.extend_from_slice(&10_000_000_000u64.to_le_bytes());
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(user.pubkey(), true),
                AccountMeta::new(pool_pda, false),
                AccountMeta::new(user_ta_a, false),
                AccountMeta::new(user_ta_b, false),
                AccountMeta::new(pool_accs.reserve_a, false),
                AccountMeta::new(pool_accs.reserve_b, false),
                AccountMeta::new(pool_accs.lp_mint, false),
                AccountMeta::new(user_lp, false),
                AccountMeta::new_readonly(token_id(), false),
            ],
            data: add_data,
        }],
        Some(&user.pubkey()), &[&user], svm.latest_blockhash(),
    )).unwrap();

    // Swap should fail with ERR_CB_TRIGGERED (amount >> threshold)
    let mut swap_data = vec![1u8];
    swap_data.extend_from_slice(&1_000_000_000u64.to_le_bytes());
    swap_data.extend_from_slice(&0u64.to_le_bytes());

    let swap_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(user.pubkey(), true),
            AccountMeta::new(pool_pda, false),
            AccountMeta::new(user_ta_a, false),
            AccountMeta::new(user_ta_b, false),
            AccountMeta::new(pool_accs.reserve_a, false),
            AccountMeta::new(pool_accs.reserve_b, false),
            AccountMeta::new_readonly(token_id(), false),
        ],
        data: swap_data,
    };

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[swap_ix], Some(&user.pubkey()), &[&user], svm.latest_blockhash(),
    ));
    assert_error(result, 7001); // ERR_CB_TRIGGERED
}

#[test]
fn test_emergency_withdraw() {
    let (mut svm, program_id) = setup_svm();
    let authority = Keypair::new();
    let user = Keypair::new();
    airdrop(&mut svm, &authority.pubkey(), 50_000_000_000);
    airdrop(&mut svm, &user.pubkey(), 50_000_000_000);

    let mint_a = create_mint(&mut svm, &authority);
    let mint_b = create_mint(&mut svm, &authority);

    let (pool_pda, _) = init_pool(&mut svm, &program_id, &authority, &mint_a, &mint_b, 100);
    let pool_accs = create_pool_accounts(&mut svm, &authority, &mint_a, &mint_b, &pool_pda);

    let user_ta_a = create_token_account(&mut svm, &user, &user.pubkey(), &mint_a);
    let user_ta_b = create_token_account(&mut svm, &user, &user.pubkey(), &mint_b);
    let user_lp = create_token_account(&mut svm, &user, &user.pubkey(), &pool_accs.lp_mint);

    let amt_a: u64 = 5_000_000_000;
    let amt_b: u64 = 5_000_000_000;
    mint_tokens(&mut svm, &authority, &mint_a, &user_ta_a, amt_a);
    mint_tokens(&mut svm, &authority, &mint_b, &user_ta_b, amt_b);

    // Add liquidity
    let mut add_data = vec![2u8];
    add_data.extend_from_slice(&amt_a.to_le_bytes());
    add_data.extend_from_slice(&amt_b.to_le_bytes());
    svm.send_transaction(Transaction::new_signed_with_payer(
        &[Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(user.pubkey(), true),
                AccountMeta::new(pool_pda, false),
                AccountMeta::new(user_ta_a, false),
                AccountMeta::new(user_ta_b, false),
                AccountMeta::new(pool_accs.reserve_a, false),
                AccountMeta::new(pool_accs.reserve_b, false),
                AccountMeta::new(pool_accs.lp_mint, false),
                AccountMeta::new(user_lp, false),
                AccountMeta::new_readonly(token_id(), false),
            ],
            data: add_data,
        }],
        Some(&user.pubkey()), &[&user], svm.latest_blockhash(),
    )).unwrap();

    assert_eq!(get_token_balance(&svm, &pool_accs.reserve_a), amt_a);
    assert_eq!(get_token_balance(&svm, &pool_accs.reserve_b), amt_b);

    // Authority destination accounts
    let auth_ta_a = create_token_account(&mut svm, &authority, &authority.pubkey(), &mint_a);
    let auth_ta_b = create_token_account(&mut svm, &authority, &authority.pubkey(), &mint_b);

    // Emergency withdraw
    let emergency_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(authority.pubkey(), true),
            AccountMeta::new(pool_pda, false),
            AccountMeta::new(pool_accs.reserve_a, false),
            AccountMeta::new(pool_accs.reserve_b, false),
            AccountMeta::new(auth_ta_a, false),
            AccountMeta::new(auth_ta_b, false),
            AccountMeta::new_readonly(token_id(), false),
        ],
        data: vec![5u8],
    };

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[emergency_ix],
        Some(&authority.pubkey()),
        &[&authority],
        svm.latest_blockhash(),
    ));
    assert!(result.is_ok(), "Emergency withdraw failed: {:?}", result.err());

    // Reserves should be empty
    assert_eq!(get_token_balance(&svm, &pool_accs.reserve_a), 0, "Reserve A should be drained");
    assert_eq!(get_token_balance(&svm, &pool_accs.reserve_b), 0, "Reserve B should be drained");

    // Authority received the funds
    assert_eq!(get_token_balance(&svm, &auth_ta_a), amt_a);
    assert_eq!(get_token_balance(&svm, &auth_ta_b), amt_b);
}

#[test]
fn test_unauthorized_pause_fails() {
    let (mut svm, program_id) = setup_svm();
    let authority = Keypair::new();
    let attacker = Keypair::new();
    airdrop(&mut svm, &authority.pubkey(), 50_000_000_000);
    airdrop(&mut svm, &attacker.pubkey(), 10_000_000_000);

    let mint_a = create_mint(&mut svm, &authority);
    let mint_b = create_mint(&mut svm, &authority);
    let (pool_pda, _) = init_pool(&mut svm, &program_id, &authority, &mint_a, &mint_b, 100);

    // Attacker tries to pause
    let pause_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(attacker.pubkey(), true),
            AccountMeta::new(pool_pda, false),
        ],
        data: vec![3u8, 1u8],
    };

    let result = svm.send_transaction(Transaction::new_signed_with_payer(
        &[pause_ix], Some(&attacker.pubkey()), &[&attacker], svm.latest_blockhash(),
    ));
    assert_error(result, 7002); // ERR_UNAUTHORIZED

    // Pool should still be unpaused
    let pool_data = get_account_data(&svm, &pool_pda);
    assert_eq!(pool_data[204], 0, "Pool should still be unpaused");
}
