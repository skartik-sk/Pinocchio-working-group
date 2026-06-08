# p-ATA recover_nested — CU Comparison

> Legacy and p-ATA numbers from [SIMD #543](https://github.com/solana-foundation/solana-improvement-documents/discussions/543).

## Results

| Instruction | Legacy | p-ATA | **Ours** | Reduction vs Legacy |
|---|---|---|---|---|
| recover_nested (owner=spl, nested=spl) | 26,806 | 5,191 | **4,650** | −82.7% |
| recover_nested (owner=token-2022, nested=token-2022) | — | — | **7,235** | — |
| recover_nested (owner=spl, nested=token-2022) | — | — | **7,988** | — |
| recover_nested (owner=token-2022, nested=spl) | — | — | **4,659** | — |

> Legacy/p-ATA numbers for mixed and Token-2022 combinations are not publicly available.

## Key optimizations

| What | How | CU saved |
|---|---|---|
| Single PDA derivation for signing | Derive `owner_ata` once, reuse bump for both `TransferChecked` + `CloseAccount` CPIs | ~800 CU |
| No redundant ownership checks | Validate `owner_ata` ownership once, then sign all CPIs with same seeds | ~200 CU |
| Optional nested token program | Falls back to `owner_token_program` when both are the same — avoids extra account processing | ~100 CU |
| Inline seed construction | `seeds!` macro compiles to stack array — zero heap allocation | ~50 CU |
