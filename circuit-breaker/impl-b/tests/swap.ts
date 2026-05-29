import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { StableSwapCircuitBreaker } from "../target/types/stable_swap_circuit_breaker";
import { PublicKey, Keypair, SystemProgram } from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { assert } from "chai";

describe("stable-swap-circuit-breaker", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.StableSwapCircuitBreaker as Program<StableSwapCircuitBreaker>;

  let authority: anchor.web3.Keypair;
  let user: anchor.web3.Keypair;
  let mintA: PublicKey;
  let mintB: PublicKey;
  let poolState: PublicKey;
  let reserveA: PublicKey;
  let reserveB: PublicKey;
  let lpMint: PublicKey;

  beforeEach(async () => {
    authority = anchor.web3.Keypair.generate();
    user = anchor.web3.Keypair.generate();

    await provider.connection.confirmTransaction(
      await provider.connection.requestAirdrop(authority.publicKey, 10 * anchor.web3.LAMPORTS_PER_SOL),
      "confirmed"
    );

    await provider.connection.confirmTransaction(
      await provider.connection.requestAirdrop(user.publicKey, 10 * anchor.web3.LAMPORTS_PER_SOL),
      "confirmed"
    );

    mintA = await createMint(provider.connection, authority, authority.publicKey, null, 9);
    mintB = await createMint(provider.connection, authority, authority.publicKey, null, 9);

    const [poolPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("stable-swap-pool"), mintA.toBuffer(), mintB.toBuffer()],
      program.programId
    );
    poolState = poolPda;

    const vaultA = Keypair.generate();
    const vaultB = Keypair.generate();
    const lpMintKeypair = Keypair.generate();
    reserveA = vaultA.publicKey;
    reserveB = vaultB.publicKey;
    lpMint = lpMintKeypair.publicKey;
  });

  it("initializes pool", async () => {
    await program.methods
      .initPool(new anchor.BN(100))
      .accounts({
        authority: authority.publicKey,
        mintA,
        mintB,
        poolState,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([authority])
      .rpc();

    const pool = await program.account.poolState.fetch(poolState);
    assert.strictEqual(pool.ampFactor.toNumber(), 100);
    assert.strictEqual(pool.paused, false);
    assert.strictEqual(pool.cbWindowSec.toNumber(), 60);
  });

  it("pauses pool", async () => {
    await program.methods
      .initPool(new anchor.BN(100))
      .accounts({
        authority: authority.publicKey,
        mintA,
        mintB,
        poolState,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([authority])
      .rpc();

    await program.methods
      .setPause(true)
      .accounts({
        authority: authority.publicKey,
        poolState,
      })
      .signers([authority])
      .rpc();

    const pool = await program.account.poolState.fetch(poolState);
    assert.strictEqual(pool.paused, true);
  });

  it("prevents swap when paused", async () => {
    await program.methods
      .initPool(new anchor.BN(100))
      .accounts({
        authority: authority.publicKey,
        mintA,
        mintB,
        poolState,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([authority])
      .rpc();

    await program.methods
      .setPause(true)
      .accounts({
        authority: authority.publicKey,
        poolState,
      })
      .signers([authority])
      .rpc();

    const userAtaA = await createAccount(provider.connection, user, mintA, user.publicKey);
    const userAtaB = await createAccount(provider.connection, user, mintB, user.publicKey);
    await mintTo(provider.connection, authority, mintA, userAtaA, authority, 1_000_000);

    try {
      await program.methods
        .swap(new anchor.BN(100_000), new anchor.BN(0))
        .accounts({
          user: user.publicKey,
          poolState,
          userSource: userAtaA,
          userDest: userAtaB,
          reserveIn: reserveA,
          reserveOut: reserveB,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([user])
        .rpc();

      assert.fail("Should have thrown");
    } catch (err) {
      assert.include(err.toString(), "7000");
    }
  });

  it("executes swap", async () => {
    await program.methods
      .initPool(new anchor.BN(100))
      .accounts({
        authority: authority.publicKey,
        mintA,
        mintB,
        poolState,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([authority])
      .rpc();

    const userAtaA = await createAccount(provider.connection, user, mintA, user.publicKey);
    const userAtaB = await createAccount(provider.connection, user, mintB, user.publicKey);
    await mintTo(provider.connection, authority, mintA, userAtaA, authority, 1_000_000);

    const userAtaBBefore = await getAccount(provider.connection, userAtaB);

    await program.methods
      .swap(new anchor.BN(100_000), new anchor.BN(90_000))
      .accounts({
        user: user.publicKey,
        poolState,
        userSource: userAtaA,
        userDest: userAtaB,
        reserveIn: reserveA,
        reserveOut: reserveB,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([user])
      .rpc();

    const userAtaBAfter = await getAccount(provider.connection, userAtaB);

    assert.ok(parseInt(userAtaBAfter.amount.toString()) > parseInt(userAtaBBefore.amount.toString()));
  });

  it("triggers circuit breaker", async () => {
    await program.methods
      .initPool(new anchor.BN(100))
      .accounts({
        authority: authority.publicKey,
        mintA,
        mintB,
        poolState,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([authority])
      .rpc();

    await program.methods
      .updateCb(new anchor.BN(60), new anchor.BN(500_000))
      .accounts({
        authority: authority.publicKey,
        poolState,
      })
      .signers([authority])
      .rpc();

    const userAtaA = await createAccount(provider.connection, user, mintA, user.publicKey);
    const userAtaB = await createAccount(provider.connection, user, mintB, user.publicKey);
    await mintTo(provider.connection, authority, mintA, userAtaA, authority, 1_000_000);

    try {
      await program.methods
        .swap(new anchor.BN(600_000), new anchor.BN(0))
        .accounts({
          user: user.publicKey,
          poolState,
          userSource: userAtaA,
          userDest: userAtaB,
          reserveIn: reserveA,
          reserveOut: reserveB,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([user])
        .rpc();

      assert.fail("Should have thrown");
    } catch (err) {
      assert.include(err.toString(), "7001");
    }
  });
});
