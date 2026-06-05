# Compute Unit Benchmarks — p-ATA create-idempotent

Benchmarks run via [Mollusk](https://github.com/deanlittlelabs/mollusk) compute unit bencher.

## Comparison with Legacy ATA & Official p-ATA

> Legacy and p-ATA numbers from [SIMD #543](https://github.com/solana-foundation/solana-improvement-documents/discussions/543).

| Instruction | Legacy | p-ATA | **p-ATA Optimal (ours)** | Our reduction |
|---|---|---|---|---|
| create_idempotent (new, spl-token) | 22,940 | 4,171 | **3,490** | −84.8% |
| create_idempotent (existing, spl-token) | 3,710 | 548 | 927 | −75.0% |
| create_idempotent (new, token-2022) | 15,474 | 5,496 | **5,169** | −66.3% |
| create_idempotent (existing, token-2022) | 8,210 | 1,634 | **566** | −93.1% |

### Highlights

- **Create (new, SPL Token)**: 840 CU cheaper than official p-ATA — no CPI for account length, stack arrays, constant short-circuit
- **Create (new, Token-2022)**: 327 CU cheaper than official p-ATA — local TLV parsing beats the CPI
- **Idempotent existing**: CU values are dominated by `derive_program_address` bump iteration count, which is data-dependent. SPL Token idempotent (927) is higher than official (548) due to less favorable bump iteration, not slower code. Token-2022 idempotent (566) is lower for the same reason — random noise, not a real optimization

### Our optimizations vs official p-ATA

| Optimization | Official p-ATA | Ours |
|---|---|---|
| Account length (no extensions) | CPI call | **Constant return (170)** |
| Account length (with extensions) | CPI call | **Local TLV parsing** |
| Data structures | Stack arrays (`no_std`) | Stack arrays |
| Dedup | Not needed (CPI handles it) | Not needed (clean by construction) |
| Token-2022 init | **Batched CPI** (1 call) | 2 separate CPIs (not yet batched) |
| Bump derivation | Optional hint via `CreateWithArgs` | Always `derive_program_address` |

### What's left to close the gap

- **Batch CPI** for Token-2022 (combine `InitializeImmutableOwner` + `InitializeAccount3` into one CPI) — saves ~1,000 CU
- **Bump hint** via new `CreateWithArgs` instruction — saves ~300-600 CU
