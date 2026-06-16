# WarpXSwap Test Suite

Bankrun-based TypeScript tests for the WarpXSwap on-chain programs.

## Prerequisites

```bash
# 1. Build the Anchor programs (generates target/idl/*.json and target/types/*.ts)
anchor build

# 2. Install test dependencies
cd WarpXSwap/tests
npm install
```

## Running tests

```bash
# All tests
npm test

# Individual test groups
npm run test:weight     # pool_weight accounting (bug fix regression)
npm run test:fees       # pool-wide fps accumulator model
npm run test:swap       # swap instruction + oracle/fee/concentration math
npm run test:deposits   # deposit / withdraw / claim / compound flows
```

Or via Anchor from the workspace root:

```bash
anchor test                         # runs all test groups
anchor test -- --grep "pool_weight" # filter by name
```

## Test file layout

| File | Covers |
|------|--------|
| `pool-weight.test.ts` | pool_weight bug fix regressions (#1, #8): starts at 0, deposit doesn't touch it, public_exit subtracts fee_share only |
| `pool-fee-accumulator.test.ts` | pool_fps formula, LP fee_debt, pending_fees on re-deposit, capital switching, compound dilution trade-off |
| `pool-swap.test.ts` | Oracle-rate math, outgoing-only fee, max % concentration guard, slippage, pool_fps after swap |
| `pool-deposit-withdraw.test.ts` | All deposit/withdraw/exit state invariants, partial exit, claim_fees, compound_fees |

## Math helpers in `helpers/setup.ts`

These mirror the exact on-chain formulas from `swap.rs` and `deposit_withdraw.rs`:

```typescript
computeFpsIncrement(outFeeAmount, poolTotalLpDeposited)
  → Δfps = outFeeAmount × FEE_SCALE / poolTotalLpDeposited

computeClaimable(lpAmount, poolFps, feeDebt, pendingFees)
  → claimable = pendingFees + lpAmount × (poolFps − feeDebt) / FEE_SCALE

computeOutFee(amountOutBeforeFee, feeBps)
  → out_fee = amountOutBeforeFee × feeBps / 10_000
```

## Notes

- Tests that exercise the full instruction set (deposit/swap transactions) require
  `anchor build` to generate `target/idl/pool_program.json`.
- Math-only invariant tests (most tests in the suite) run without a live validator —
  they verify the on-chain logic by computing expected values in TypeScript.
- Bankrun tests run significantly faster than `solana-test-validator` (~10× speedup).
