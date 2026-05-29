import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { EscrowCircuitBreaker } from "../target/types/escrow_circuit_breaker";
import { PublicKey, Keypair, SystemProgram } from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { assert } from "chai";

describe("escrow-circuit-breaker", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.EscrowCircuitBreaker as Program<EscrowCircuitBreaker>;

  let maker: anchor.web3.Keypair;
  let taker: anchor.web3.Keypair;
  let mintA: PublicKey;
  let mintB: PublicKey;
  let makerAtaA: PublicKey;
  let makerAtaB: PublicKey;
  let takerAtaA: PublicKey;
  let takerAtaB: PublicKey;
  let escrow: PublicKey;
  let vault: PublicKey;
  let cbPda: PublicKey;

  const AMOUNT = 1_000_000;

  beforeEach(async () => {
    maker = anchor.web3.Keypair.generate();
    taker = anchor.web3.Keypair.generate();

    await provider.connection.confirmTransaction(
      await provider.connection.requestAirdrop(maker.publicKey, 10 * anchor.web3.LAMPORTS_PER_SOL),
      "confirmed"
    );

    await provider.connection.confirmTransaction(
      await provider.connection.requestAirdrop(taker.publicKey, 10 * anchor.web3.LAMPORTS_PER_SOL),
      "confirmed"
    );

    mintA = await createMint(provider.connection, maker, maker.publicKey, null, 9);
    mintB = await createMint(provider.connection, maker, maker.publicKey, null, 9);

    makerAtaA = await createAccount(provider.connection, maker, mintA, maker.publicKey);
    makerAtaB = await createAccount(provider.connection, maker, mintB, maker.publicKey);
    takerAtaA = await createAccount(provider.connection, taker, mintA, taker.publicKey);
    takerAtaB = await createAccount(provider.connection, taker, mintB, taker.publicKey);

    await mintTo(provider.connection, maker, mintA, makerAtaA, maker, 100_000_000);
    await mintTo(provider.connection, maker, mintB, makerAtaB, maker, 100_000_000);
    await mintTo(provider.connection, maker, mintB, takerAtaB, maker, 100_000_000);

    const [escrowPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("escrow"), maker.publicKey.toBuffer()],
      program.programId
    );
    escrow = escrowPda;

    const [cbPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("circuit-breaker"), maker.publicKey.toBuffer()],
      program.programId
    );
    cbPda = cbPda;
  });

  it("initializes circuit breaker", async () => {
    const tx = await program.methods
      .initCircuitBreaker(new anchor.BN(60), 1, new anchor.BN(1_000_000))
      .accounts({
        authority: maker.publicKey,
        circuitBreaker: cbPda,
        systemProgram: SystemProgram.programId,
      })
      .signers([maker])
      .rpc();

    await provider.connection.confirmTransaction(tx, "confirmed");

    const cbAccount = await program.account.circuitBreaker.fetch(cbPda);
    assert.strictEqual(cbAccount.paused, false);
    assert.strictEqual(cbAccount.config.windowSeconds.toNumber(), 60);
    assert.strictEqual(cbAccount.config.thresholdType, 1);
    assert.strictEqual(cbAccount.config.threshold.toNumber(), 1_000_000);
  });

  it("creates escrow", async () => {
    await program.methods
      .initCircuitBreaker(new anchor.BN(60), 1, new anchor.BN(1_000_000))
      .accounts({
        authority: maker.publicKey,
        circuitBreaker: cbPda,
        systemProgram: SystemProgram.programId,
      })
      .signers([maker])
      .rpc();

    const vaultKeypair = Keypair.generate();

    const tx = await program.methods
      .make(new anchor.BN(AMOUNT))
      .accounts({
        maker: maker.publicKey,
        mintA,
        mintB,
        makerAtaA,
        vault: vaultKeypair.publicKey,
        escrow,
        circuitBreaker: cbPda,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([maker, vaultKeypair])
      .rpc();

    await provider.connection.confirmTransaction(tx, "confirmed");

    const vaultAccount = await getAccount(provider.connection, vaultKeypair.publicKey);
    assert.strictEqual(parseInt(vaultAccount.amount.toString()), AMOUNT);
  });

  it("pauses circuit breaker", async () => {
    await program.methods
      .initCircuitBreaker(new anchor.BN(60), 1, new anchor.BN(1_000_000))
      .accounts({
        authority: maker.publicKey,
        circuitBreaker: cbPda,
        systemProgram: SystemProgram.programId,
      })
      .signers([maker])
      .rpc();

    await program.methods
      .setPause(true)
      .accounts({
        authority: maker.publicKey,
        circuitBreaker: cbPda,
      })
      .signers([maker])
      .rpc();

    const cbAccount = await program.account.circuitBreaker.fetch(cbPda);
    assert.strictEqual(cbAccount.paused, true);
  });

  it("prevents escrow when paused", async () => {
    await program.methods
      .initCircuitBreaker(new anchor.BN(60), 1, new anchor.BN(1_000_000))
      .accounts({
        authority: maker.publicKey,
        circuitBreaker: cbPda,
        systemProgram: SystemProgram.programId,
      })
      .signers([maker])
      .rpc();

    await program.methods
      .setPause(true)
      .accounts({
        authority: maker.publicKey,
        circuitBreaker: cbPda,
      })
      .signers([maker])
      .rpc();

    const vaultKeypair = Keypair.generate();

    try {
      await program.methods
        .make(new anchor.BN(AMOUNT))
        .accounts({
          maker: maker.publicKey,
          mintA,
          mintB,
          makerAtaA,
          vault: vaultKeypair.publicKey,
          escrow,
          circuitBreaker: cbPda,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([maker, vaultKeypair])
        .rpc();

      assert.fail("Should have thrown error");
    } catch (err) {
      assert.include(err.toString(), "6001");
    }
  });

  it("takes escrow successfully", async () => {
    await program.methods
      .initCircuitBreaker(new anchor.BN(60), 1, new anchor.BN(1_000_000))
      .accounts({
        authority: maker.publicKey,
        circuitBreaker: cbPda,
        systemProgram: SystemProgram.programId,
      })
      .signers([maker])
      .rpc();

    const vaultKeypair = Keypair.generate();

    await program.methods
      .make(new anchor.BN(AMOUNT))
      .accounts({
        maker: maker.publicKey,
        mintA,
        mintB,
        makerAtaA,
        vault: vaultKeypair.publicKey,
        escrow,
        circuitBreaker: cbPda,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([maker, vaultKeypair])
      .rpc();

    const makerAtaABefore = await getAccount(provider.connection, makerAtaA);

    await program.methods
      .take(new anchor.BN(AMOUNT), new anchor.BN(0))
      .accounts({
        taker: taker.publicKey,
        maker: maker.publicKey,
        mintA,
        mintB,
        makerAtaA,
        takerAtaA,
        takerAtaB,
        vault: vaultKeypair.publicKey,
        escrow,
        circuitBreaker: cbPda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([taker])
      .rpc();

    const makerAtaAAfter = await getAccount(provider.connection, makerAtaA);

    assert.strictEqual(
      parseInt(makerAtaAAfter.amount.toString()) - parseInt(makerAtaABefore.amount.toString()),
      AMOUNT
    );
  });
});
