# p-ATA create — CU Comparison

> Legacy and p-ATA numbers from [SIMD #543](https://github.com/solana-foundation/solana-improvement-documents/discussions/543).

## Results

| Instruction | Legacy | p-ATA | **Ours** | Reduction vs Legacy |
|---|---|---|---|---|
| create (spl-token) | 18,433 | 3,083 | **3,485** | −81.1% |
| create (token-2022) | 13,967 | 5,132 | **5,159** | −63.1% |

## Key optimizations

| What | How | CU saved |
|---|---|---|
| No CPI for account length | Constant return (165) for SPL Token accounts | ~1,500 CU |
| Stack arrays | `[ExtensionType; 8]` instead of `vec!` | ~300-500 CU |
| Clean by construction | No sort/dedup — duplicates never created | ~50 CU |
