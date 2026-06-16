/**
 * pool-usd-fee-claim.test.ts
 *
 * Integration tests for Bug #2 — USD-normalised fee claim path.
 *
 * Problem (pre-fix):
 *   pool_fps was incremented and claimed using NATIVE token amounts as
 *   the denominator/numerator.  Because SOL uses 9 decimals and USDC
 *   uses 6 decimals, an LP who deposited 1 SOL (1e9 lamports) held 1000×
 *   more native units than an LP who deposited 150 USDC (150e6) for the
 *   exact same $150 USD value.  The SOL LP therefore captured ~1000× more
 *   fee share than the USDC LP despite equal economic contribution.
 *
 * Fix (post-fix):
 *   All amounts are normalised to USD at deposit and swap time:
 *     amount_usd = amount × oracle_price / (ORACLE_PRICE_SCALE × 10^decimals)
 *   pool_fps accumulator and fee-claim formula use amount_usd, not amount.
 *   Final payout converts back: native = usd × ORACLE_PRICE_SCALE × 10^dec / price
 *
 * Test scenarios:
 *   1.  computeAmountUsd — SOL and USDC at equal USD value normalise identically
 *   2.  computeAmountUsd — different USD values produce proportional results
 *   3.  oracle_price = 0 fallback — amount_usd falls back to native amount
 *   4.  computeFeeUsd — outgoing fee converted to USD correctly
 *   5.  computeFpsIncrementUsd — fps increment uses USD-normalised denominator
 *   6.  Equal USD LPs earn equal fee shares (core Bug #2 regression)
 *   7.  Unequal USD LPs earn proportional fee shares
 *   8.  Pre-fix (native denominator) would have given WRONG unequal shares
 *   9.  computeClaimableUsd — pending_fees carry through re-deposit
 *   10. computeClaimableNative — converts USD claimable back to native SOL
 *   11. computeClaimableNative — converts USD claimable back to native USDC
 *   12. Full end-to-end: two LPs with equal USD value deposit, swap, claim equally
 *   13. fee_debt prevents retroactive earnings (USD model)
 *   14. pool_total_lp_deposited_usd decreases on LP exit
 *   15. Multi-swap accumulation — fps increments are additive across swaps
 */

import { expect } from "chai";
import { describe, it, before } from "mocha";
import {
  createTestContext,
  FEE_SCALE,
  ORACLE_PRICE_SCALE,
  computeAmountUsd,
  computeFeeUsd,
  computeFpsIncrementUsd,
  computeClaimableUsd,
  computeClaimableNative,
  computeFpsIncrement,
  computeClaimable,
  TestCtx,
} from "./helpers/setup";

// ── Asset parameters used across tests ────────────────────────
// SOL: 9 decimals, $150 per SOL
const SOL_DECIMALS    = 9;
const SOL_PRICE       = 150_000_000n; // $150.00 × ORACLE_PRICE_SCALE (1e6)

// USDC: 6 decimals, $1 per USDC
const USDC_DECIMALS   = 6;
const USDC_PRICE      = 1_000_000n;  // $1.00 × ORACLE_PRICE_SCALE (1e6)

// 1 SOL = $150 USD.  150 USDC = $150 USD.  Equal economic value.
const ONE_SOL         = 1_000_000_000n; // 1e9 lamports
const ONE_FIFTY_USDC  = 150_000_000n;   // 150e6 micro-USDC

describe("Bug #2 — USD-normalised fee claim path", () => {
  let ctx: TestCtx;

  before(async () => {
    ctx = await createTestContext();
  });

  // ── 1. USD normalisation at deposit ───────────────────────────

  describe("computeAmountUsd — deposit normalisation", () => {
    it("1 SOL at $150 normalises to 150 USD units", () => {
      // amount_usd = 1e9 × 150_000_000 / (1_000_000 × 1e9) = 150
      const amountUsd = computeAmountUsd(ONE_SOL, SOL_PRICE, SOL_DECIMALS);
      expect(amountUsd).to.equal(150n,
        "1 SOL at $150 should normalise to exactly 150 USD units");
    });

    it("150 USDC at $1 normalises to 150 USD units", () => {
      // amount_usd = 150e6 × 1_000_000 / (1_000_000 × 1e6) = 150
      const amountUsd = computeAmountUsd(ONE_FIFTY_USDC, USDC_PRICE, USDC_DECIMALS);
      expect(amountUsd).to.equal(150n,
        "150 USDC at $1 should normalise to exactly 150 USD units");
    });

    it("SOL and USDC at equal USD value produce identical amount_usd", () => {
      const solUsd  = computeAmountUsd(ONE_SOL,        SOL_PRICE,  SOL_DECIMALS);
      const usdcUsd = computeAmountUsd(ONE_FIFTY_USDC, USDC_PRICE, USDC_DECIMALS);
      expect(solUsd).to.equal(usdcUsd,
        "Equal USD value → identical amount_usd regardless of decimal difference");
    });

    it("different USD values produce proportional amount_usd", () => {
      // 1 SOL = $150, 0.5 SOL = $75
      const half_SOL   = 500_000_000n;
      const fullUsd    = computeAmountUsd(ONE_SOL,  SOL_PRICE, SOL_DECIMALS);
      const halfUsd    = computeAmountUsd(half_SOL, SOL_PRICE, SOL_DECIMALS);
      expect(fullUsd).to.equal(halfUsd * 2n,
        "Double the deposit → double the amount_usd");
    });

    it("oracle_price = 0 falls back to native amount (no oracle yet)", () => {
      // Before InfoPool ever pushes a price, oracle_price = 0.
      // Fallback: amount_usd = native amount (preserves some ordering but
      // is not cross-asset comparable — acceptable for genesis state).
      const fallback = computeAmountUsd(ONE_SOL, 0n, SOL_DECIMALS);
      expect(fallback).to.equal(ONE_SOL,
        "Zero oracle price must fall back to native amount");
    });
  });

  // ── 2. Fee USD normalisation at swap time ─────────────────────

  describe("computeFeeUsd — outgoing fee normalisation", () => {
    it("0.01 SOL fee at $150 → $1.50 fee_usd", () => {
      // 0.01 SOL = 10_000_000 lamports
      // fee_usd = 1e7 × 150_000_000 / (1_000_000 × 1e9) = 1.5 → truncates to 1
      const outFee = 10_000_000n;
      const feeUsd = computeFeeUsd(outFee, SOL_PRICE, SOL_DECIMALS);
      expect(feeUsd).to.equal(1n,
        "0.01 SOL fee ≈ $1.50 — integer division gives 1 USD unit");
    });

    it("1.5 USDC fee at $1 → 1 fee_usd (integer truncation)", () => {
      const outFee = 1_500_000n; // 1.5 USDC in micro-USDC
      const feeUsd = computeFeeUsd(outFee, USDC_PRICE, USDC_DECIMALS);
      expect(feeUsd).to.equal(1n,
        "1.5 USDC fee → 1 USD unit after integer truncation");
    });

    it("equal native fees at equal prices produce equal fee_usd", () => {
      // 150 USDC fee vs 1 SOL fee — both worth $150 USD
      const solFee     = ONE_SOL;
      const usdcFee    = ONE_FIFTY_USDC;

      const solFeeUsd  = computeFeeUsd(solFee,  SOL_PRICE,  SOL_DECIMALS);
      const usdcFeeUsd = computeFeeUsd(usdcFee, USDC_PRICE, USDC_DECIMALS);

      expect(solFeeUsd).to.equal(usdcFeeUsd,
        "Equal USD value fees in different assets normalise to the same fee_usd");
    });
  });

  // ── 3. USD-normalised fps increment ───────────────────────────

  describe("computeFpsIncrementUsd — pool_fps accumulator", () => {
    it("fps increment = fee_usd × FEE_SCALE / pool_total_lp_deposited_usd", () => {
      const feeUsd   = 10n;   // $10 USD fee from a swap
      const totalUsd = 1000n; // $1000 USD total LP deposits

      const fpsInc = computeFpsIncrementUsd(feeUsd, totalUsd);
      // fps_inc = 10 × 1e9 / 1000 = 10_000_000
      expect(fpsInc).to.equal(10_000_000n);
    });

    it("pool_total_lp_deposited_usd = 0 gives fps_inc = 0 (no LPs)", () => {
      const fpsInc = computeFpsIncrementUsd(100n, 0n);
      expect(fpsInc).to.equal(0n,
        "Division by zero guard: no LPs → fps increment is zero");
    });

    it("larger pool dilutes individual fps increment", () => {
      const feeUsd     = 10n;
      const smallPool  = computeFpsIncrementUsd(feeUsd, 100n);   // $100 total
      const largePool  = computeFpsIncrementUsd(feeUsd, 1000n);  // $1000 total

      expect(smallPool).to.be.greaterThan(largePool,
        "Larger pool dilutes fps per LP — each LP earns less per swap");
    });
  });

  // ── 4. Core regression: equal USD LPs earn equal fees ─────────

  describe("equal USD value LPs earn equal fee shares (core Bug #2 regression)", () => {
    it("Alice (1 SOL) and Bob (150 USDC) deposit equal USD, earn equal fees", () => {
      // Setup: Alice deposits 1 SOL ($150), Bob deposits 150 USDC ($150)
      const aliceAmountUsd = computeAmountUsd(ONE_SOL,        SOL_PRICE,  SOL_DECIMALS);
      const bobAmountUsd   = computeAmountUsd(ONE_FIFTY_USDC, USDC_PRICE, USDC_DECIMALS);
      // Both = 150 USD units
      expect(aliceAmountUsd).to.equal(bobAmountUsd, "Precondition: equal USD deposits");

      // Pool state: $300 total
      const poolTotalUsd = aliceAmountUsd + bobAmountUsd; // 300n

      // Swap generates 0.01 SOL fee ($1.50)
      const outFee     = 10_000_000n; // 0.01 SOL in lamports
      const feeUsd     = computeFeeUsd(outFee, SOL_PRICE, SOL_DECIMALS); // 1n

      // pool_fps increases
      const fpsInc     = computeFpsIncrementUsd(feeUsd, poolTotalUsd);

      // Claimable (in USD) — fee_debt was 0n for both
      const aliceClaimUsd = computeClaimableUsd(aliceAmountUsd, fpsInc, 0n, 0n);
      const bobClaimUsd   = computeClaimableUsd(bobAmountUsd,   fpsInc, 0n, 0n);

      expect(aliceClaimUsd).to.equal(bobClaimUsd,
        "Equal USD value LPs must earn identical fee shares under Bug #2 fix");
    });

    it("pre-fix native formula would give WRONG, unequal shares", () => {
      // This test proves the OLD formula was broken.
      // Alice: 1 SOL = 1_000_000_000 native units
      // Bob:  150 USDC = 150_000_000 native units
      // ratio 1e9 : 150e6 = 6.67 : 1 → Alice earns 6.67× more despite equal USD

      const aliceNative    = ONE_SOL;          // 1_000_000_000
      const bobNative      = ONE_FIFTY_USDC;   // 150_000_000

      const poolTotalNative = aliceNative + bobNative; // 1_150_000_000

      // Outgoing fee in native USDC (swap: SOL → USDC, fee paid in USDC)
      const outFee      = 150_000n; // 0.15 USDC in micro-USDC
      const fpsInc      = computeFpsIncrement(outFee, poolTotalNative);

      const aliceClaim  = computeClaimable(aliceNative, fpsInc, 0n, 0n);
      const bobClaim    = computeClaimable(bobNative,   fpsInc, 0n, 0n);

      // Alice holds 1_000_000_000 / 1_150_000_000 ≈ 86.96% → massive over-allocation
      // Bob  holds   150_000_000 / 1_150_000_000 ≈ 13.04% → severe under-allocation
      expect(aliceClaim).to.not.equal(bobClaim,
        "Pre-fix: native denominator gives unequal shares for equal USD value");

      const ratio = Number(aliceClaim) / Number(bobClaim === 0n ? 1n : bobClaim);
      expect(ratio).to.be.greaterThan(5,
        "Pre-fix: Alice earns 5×+ more than Bob despite equal USD — confirms bug");
    });
  });

  // ── 5. Proportional shares for unequal deposits ───────────────

  describe("unequal USD value LPs earn proportional fee shares", () => {
    it("2× USD deposit earns 2× fees", () => {
      // Alice: 1 SOL ($150), Bob: 2 SOL ($300) — Bob has 2× USD value
      const aliceUsd = computeAmountUsd(ONE_SOL,        SOL_PRICE, SOL_DECIMALS); // 150
      const bobUsd   = computeAmountUsd(ONE_SOL * 2n,   SOL_PRICE, SOL_DECIMALS); // 300
      const totalUsd = aliceUsd + bobUsd; // 450

      const feeUsd   = 45n; // $45 fee
      const fpsInc   = computeFpsIncrementUsd(feeUsd, totalUsd);

      const aliceClaim = computeClaimableUsd(aliceUsd, fpsInc, 0n, 0n);
      const bobClaim   = computeClaimableUsd(bobUsd,   fpsInc, 0n, 0n);

      expect(bobClaim).to.equal(aliceClaim * 2n,
        "2× USD deposit earns exactly 2× the fee share");
    });
  });

  // ── 6. Native-token conversion at claim ───────────────────────

  describe("computeClaimableNative — USD → native token conversion", () => {
    it("converts claimable USD back to SOL lamports at current oracle price", () => {
      // claimable_usd = 150 → at $150/SOL → claimable_native = 1 SOL = 1e9 lamports
      // claimable_native = 150 × 1_000_000 × 1e9 / 150_000_000
      //                  = 150 × 1e15 / 150e6 = 1e9
      const claimableUsd    = 150n;
      const claimableNative = computeClaimableNative(claimableUsd, SOL_PRICE, SOL_DECIMALS);
      expect(claimableNative).to.equal(1_000_000_000n,
        "$150 worth of fees → 1 SOL at $150/SOL");
    });

    it("converts claimable USD back to USDC at current oracle price", () => {
      // claimable_usd = 150 → at $1/USDC → claimable_native = 150 USDC = 150e6
      // claimable_native = 150 × 1_000_000 × 1e6 / 1_000_000 = 150e6
      const claimableUsd    = 150n;
      const claimableNative = computeClaimableNative(claimableUsd, USDC_PRICE, USDC_DECIMALS);
      expect(claimableNative).to.equal(150_000_000n,
        "$150 worth of fees → 150 USDC at $1/USDC");
    });

    it("higher oracle price reduces native claimable (same USD value, stronger token)", () => {
      // Same claimable_usd but higher SOL price → fewer lamports needed
      const claimableUsd  = 150n;
      const at150         = computeClaimableNative(claimableUsd, 150_000_000n, SOL_DECIMALS);
      const at300         = computeClaimableNative(claimableUsd, 300_000_000n, SOL_DECIMALS);

      expect(at300).to.equal(at150 / 2n,
        "Token price doubles → half as many native tokens needed to satisfy same USD claim");
    });

    it("oracle_price = 0 returns 0 (no claim possible without a price)", () => {
      const claimableNative = computeClaimableNative(150n, 0n, SOL_DECIMALS);
      expect(claimableNative).to.equal(0n,
        "Without an oracle price native conversion is impossible — return 0");
    });
  });

  // ── 7. pending_fees carry through re-deposit ──────────────────

  describe("computeClaimableUsd — pending_fees preserved on re-deposit", () => {
    it("accrued USD fees are settled into pending_fees before fee_debt reset", () => {
      const aliceUsd   = 150n;
      const fps1       = computeFpsIncrementUsd(10n, 300n); // $10 fee, $300 pool

      // Alice accrued before re-deposit
      const accrued    = computeClaimableUsd(aliceUsd, fps1, 0n, 0n);
      expect(accrued).to.be.greaterThan(0n, "Alice accrued fees before re-deposit");

      // Re-deposit: pending = accrued, fee_debt = fps1
      const pendingAfter  = accrued;
      const feeDebtAfter  = fps1;

      // More swaps after re-deposit
      const fps2 = fps1 + computeFpsIncrementUsd(10n, 450n); // pool grew

      const totalClaimable = computeClaimableUsd(aliceUsd, fps2, feeDebtAfter, pendingAfter);

      expect(totalClaimable).to.be.greaterThan(accrued,
        "Total claimable = preserved pending + new accrual after re-deposit");
    });

    it("fee_debt prevents double-claiming the same fps window", () => {
      const aliceUsd = 150n;
      const fps      = 10_000_000n;

      // Alice claims at fps = 10_000_000, fee_debt resets to fps
      const feeDebtAfterClaim = fps;

      // No further fps increase
      const claimAfterReset = computeClaimableUsd(aliceUsd, fps, feeDebtAfterClaim, 0n);
      expect(claimAfterReset).to.equal(0n,
        "After claim, fee_debt = pool_fps — no immediate re-claim possible");
    });
  });

  // ── 8. pool_total_lp_deposited_usd decreases on LP exit ───────

  describe("pool_total_lp_deposited_usd lifecycle", () => {
    it("increases by amount_usd on deposit", () => {
      const totalBefore = 0n;
      const depositUsd  = computeAmountUsd(ONE_SOL, SOL_PRICE, SOL_DECIMALS); // 150
      const totalAfter  = totalBefore + depositUsd;

      expect(totalAfter).to.equal(150n,
        "pool_total_lp_deposited_usd += amount_usd on deposit");
    });

    it("decreases by amount_usd on LP exit", () => {
      const totalBefore  = 300n;                      // two 150-USD LPs
      const exitAmtUsd   = computeAmountUsd(ONE_SOL, SOL_PRICE, SOL_DECIMALS); // 150
      const totalAfter   = totalBefore - exitAmtUsd;

      expect(totalAfter).to.equal(150n,
        "pool_total_lp_deposited_usd -= amount_usd on LP exit");
    });

    it("fps denominator shrinks when LP exits — remaining LP earns more per future swap", () => {
      // Before exit: $300 pool, $10 fee → fps_inc = 10 × 1e9 / 300 = 33_333_333
      const fpsBefore  = computeFpsIncrementUsd(10n, 300n);
      // After one LP exits: $150 pool, same $10 fee → fps_inc = 10 × 1e9 / 150 = 66_666_666
      const fpsAfter   = computeFpsIncrementUsd(10n, 150n);

      expect(fpsAfter).to.be.greaterThan(fpsBefore,
        "Smaller pool → larger fps increment per swap → remaining LP earns more");
    });
  });

  // ── 9. Multi-swap accumulation ────────────────────────────────

  describe("multi-swap fps accumulation", () => {
    it("pool_fps increments are additive across multiple swaps", () => {
      const poolTotalUsd = 300n; // $300 in pool

      const swap1FeeUsd = 10n;
      const swap2FeeUsd = 5n;
      const swap3FeeUsd = 20n;

      const fps1 = computeFpsIncrementUsd(swap1FeeUsd, poolTotalUsd);
      const fps2 = computeFpsIncrementUsd(swap2FeeUsd, poolTotalUsd);
      const fps3 = computeFpsIncrementUsd(swap3FeeUsd, poolTotalUsd);

      const totalFps = fps1 + fps2 + fps3;

      // LP with $150 (50% of pool) earns 50% of each fee
      const aliceUsd      = 150n;
      const aliceClaim    = computeClaimableUsd(aliceUsd, totalFps, 0n, 0n);
      const expectedClaim = (swap1FeeUsd + swap2FeeUsd + swap3FeeUsd) / 2n; // 50%

      expect(aliceClaim).to.equal(expectedClaim,
        "LP with 50% of pool earns 50% of cumulative fees across all swaps");
    });

    it("pool_fps is monotonically increasing (each swap adds to it)", () => {
      const totalUsd = 300n;
      let fps        = 0n;

      const fees = [10n, 5n, 20n, 3n, 8n];
      for (const feeUsd of fees) {
        const prevFps = fps;
        fps += computeFpsIncrementUsd(feeUsd, totalUsd);
        expect(fps).to.be.greaterThan(prevFps,
          "pool_fps must increase after every non-zero-fee swap");
      }
    });
  });

  // ── 10. Full end-to-end scenario ──────────────────────────────

  describe("full end-to-end: equal-USD deposit → swap → equal claim", () => {
    it("two LPs deposit equal USD, swap occurs, both claim equal fee shares", () => {
      // ── Deposits ────────────────────────────────────────────
      // Alice: 1 SOL at $150 oracle price
      const aliceAmount    = ONE_SOL;
      const aliceAmountUsd = computeAmountUsd(aliceAmount, SOL_PRICE, SOL_DECIMALS); // 150

      // Bob: 150 USDC at $1 oracle price
      const bobAmount    = ONE_FIFTY_USDC;
      const bobAmountUsd = computeAmountUsd(bobAmount, USDC_PRICE, USDC_DECIMALS);   // 150

      const poolTotalUsd = aliceAmountUsd + bobAmountUsd; // 300
      expect(poolTotalUsd).to.equal(300n);

      // Both LPs enter at pool_fps = 0 → fee_debt = 0 for both
      const aliceFeeDebt = 0n;
      const bobFeeDebt   = 0n;

      // ── Swap ────────────────────────────────────────────────
      // Trader swaps 1 SOL → ~150 USDC; USDC is the outgoing asset
      // USDC outgoing fee: 1% on 150 USDC = 1.5 USDC = 1_500_000 micro-USDC
      const outFeeUsdc = 1_500_000n; // 1.5 USDC fee
      const feeUsd     = computeFeeUsd(outFeeUsdc, USDC_PRICE, USDC_DECIMALS); // 1

      const fpsInc     = computeFpsIncrementUsd(feeUsd, poolTotalUsd);
      // fps_inc = 1 × 1e9 / 300 = 3_333_333n
      expect(fpsInc).to.be.greaterThan(0n);

      const poolFpsAfter = fpsInc; // started from 0

      // ── Claims ──────────────────────────────────────────────
      const aliceClaimUsd = computeClaimableUsd(aliceAmountUsd, poolFpsAfter, aliceFeeDebt, 0n);
      const bobClaimUsd   = computeClaimableUsd(bobAmountUsd,   poolFpsAfter, bobFeeDebt,   0n);

      expect(aliceClaimUsd).to.equal(bobClaimUsd,
        "End-to-end: both LPs deposited equal USD → must earn equal USD fees");

      // ── Native conversions ──────────────────────────────────
      const aliceClaimNativeSol  = computeClaimableNative(aliceClaimUsd, SOL_PRICE,  SOL_DECIMALS);
      const bobClaimNativeUsdc   = computeClaimableNative(bobClaimUsd,   USDC_PRICE, USDC_DECIMALS);

      // $0.5 each (integer truncation from 1 USD total fee)
      // Alice: 0.5 × 1e6 × 1e9 / 150e6 = 3_333_333 lamports (SOL)
      // Bob:   0.5 × 1e6 × 1e6  /  1e6 = 500_000 micro-USDC
      // Both represent $0.5 USD — different native amounts, same economic value
      expect(aliceClaimNativeSol).to.be.greaterThan(0n,
        "Alice receives a positive SOL fee share");
      expect(bobClaimNativeUsdc).to.be.greaterThan(0n,
        "Bob receives a positive USDC fee share");

      // Verify both represent the same USD value
      const aliceValueUsd = computeFeeUsd(aliceClaimNativeSol, SOL_PRICE,  SOL_DECIMALS);
      const bobValueUsd   = computeFeeUsd(bobClaimNativeUsdc,  USDC_PRICE, USDC_DECIMALS);
      expect(aliceValueUsd).to.equal(bobValueUsd,
        "Native claims in different tokens represent the same USD value");
    });
  });
});
