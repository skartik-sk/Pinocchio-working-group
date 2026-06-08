# p-ATA create-idempotent — CU Comparison

> Legacy and p-ATA numbers from [SIMD #543](https://github.com/solana-foundation/solana-improvement-documents/discussions/543).

## Results

| Instruction | Legacy | p-ATA | **p-ATA Optimal (ours)** | Reduction vs Legacy |
|---|---|---|---|---|
| create_idempotent (new, spl-token) | 22,940 | 4,171 | **3,490** | −84.8% |
| create_idempotent (existing, spl-token) | 3,710 | 548 | 927 | −75.0% |
| create_idempotent (new, token-2022) | 15,474 | 5,496 | **5,169** | −66.3% |
| create_idempotent (existing, token-2022) | 8,210 | 1,634 | **566** | −93.1% |

### Weighted average (75% SPL Token traffic): **−81.2%**

## Where we beat the official p-ATA

| Path | Official p-ATA | Ours | Why |
|---|---|---|---|
| create (new, spl-token) | 4,171 | **3,490** (−681) | No CPI for account length |
| create (new, token-2022) | 5,496 | **5,169** (−327) | Local TLV parsing beats CPI |

## Where official p-ATA beats us

| Path | Official p-ATA | Ours | Why |
|---|---|---|---|
| idempotent (existing, spl-token) | 548 | 927 (+379) | Bump iteration noise + missing batch CPI |
| idempotent (existing, token-2022) | 1,634 | 566 (−1,068) | Bump iteration noise — not a real win |

## Key optimizations in our implementation

| What | How | CU saved |
|---|---|---|
| No CPI for account length | Local TLV parsing + constant short-circuit | ~1,500 CU |
| Stack arrays | `[ExtensionType; 8]` instead of `vec!` | ~300-500 CU |
| Constant return | `TOKEN_2022_BASE_ACCOUNT_DATA_SIZE` (170) for plain mints | ~200 CU |
| Clean by construction | No sort/dedup needed — duplicates never created | ~50 CU |

## Remaining optimizations

| What | How | Est. CU saved |
|---|---|---|
| Batch CPI | Combine `InitializeImmutableOwner` + `InitializeAccount3` into one CPI | ~1,000 CU |
| Bump hint | New `CreateWithArgs` instruction with bump from caller | ~300-600 CU |
