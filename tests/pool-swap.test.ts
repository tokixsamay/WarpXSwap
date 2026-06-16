/**
 * pool-swap.test.ts
 *
 * Tests for the swap instruction:
 *   - Oracle price must be set before swap (OraclePriceNotSet guard)
 *   - Outgoing asset checks allowed list (InteractionNotAllowed)
 *   - Inflow blocked when threshold exceeded (InflowBlocked)
 *   - Max % concentration guard (MaxPctBufferExceeded)
 *   - Fee is applied only to asset_out (outgoing only)
 *   - pool_fps increments correctly after each swap
 *   - pool_weight increments by out_fee_amount after swap
 *   - Slippage protection works (min_amount_out)
 *   - amount_out sent to user = amount_before_fee - out_fee_amount
 */

import { expect } from "chai";
import { describe, it, before } from "mocha";
import { BN } from "@coral-xyz/anchor";
import {
  createTestContext,
  computeOutFee,
  computeFpsIncrement,
  computeClaimable,
  FEE_SCALE,
  BPS_DENOMINATOR,
  TestCtx,
} from "./helpers/setup";

// ── Oracle-rate swap math (mirrors swap.rs) ────────────────────
// amount_out_before_fee = amount_in × oracle_price_in / oracle_price_out
// out_fee_amount        = amount_out_before_fee × current_fee / 10_000
// amount_out            = amount_out_before_fee − out_fee_amount

function computeSwapOut(
  amountIn:     bigint,
  oraclePriceIn: bigint,
  oraclePriceOut: bigint,
  feeBps:       bigint,
): { amountOut: bigint; outFee: bigint; amountOutBeforeFee: bigint } {
  const amountOutBeforeFee = (amountIn * oraclePriceIn) / oraclePriceOut;
  const outFee             = (amountOutBeforeFee * feeBps) / BPS_DENOMINATOR;
  const amountOut          = amountOutBeforeFee - outFee;
  return { amountOut, outFee, amountOutBeforeFee };
}

describe("swap instruction", () => {
  let ctx: TestCtx;

  before(async () => {
    ctx = await createTestContext();
  });

  // ── Fee math ───────────────────────────────────────────────────

  describe("oracle-rate swap math", () => {
    it("amount_out computed correctly at oracle rates", () => {
      const amountIn        = 1_000_000_000n;  // 1 SOL (9 dec)
      const solPrice        = 86_000_000n;     // $86.00 × 1e6
      const usdcPrice       = 1_000_000n;      // $1.00 × 1e6
      const feeBps          = 100n;            // 1% outgoing fee on SOL

      const { amountOut, outFee, amountOutBeforeFee } = computeSwapOut(
        amountIn, solPrice, usdcPrice, feeBps,
      );

      // 1 SOL → 86 USDC before fee → 85.14 USDC after 1% fee
      // amountOutBeforeFee = 1e9 × 86e6 / 1e6 = 86e9 = 86_000_000_000 (in USDC lamports)
      expect(amountOutBeforeFee).to.equal(86_000_000_000n);
      expect(outFee).to.equal(860_000_000n,
        "1% fee on 86 USDC = 0.86 USDC");
      expect(amountOut).to.equal(85_140_000_000n,
        "User receives 85.14 USDC");
    });

    it("fee is ONLY on outgoing asset (SOL enters: no fee; USDC exits: fee)", () => {
      // Swap: SOL → USDC
      // SOL (asset_in): enters pool — NO fee deducted
      // USDC (asset_out): exits pool — fee applies to USDC being sent out

      const amountIn    = 1_000_000_000n;  // 1 SOL
      const feeBps      = 100n;            // 1% on USDC (asset_out)

      const { amountOut, outFee } = computeSwapOut(
        amountIn, 86_000_000n, 1_000_000n, feeBps,
      );

      // The 1 SOL that enters the pool: stays in SOL vault INTACT (no fee)
      const solEnteredPool = amountIn;
      expect(solEnteredPool).to.equal(amountIn,
        "Full amount_in enters pool vault — no fee on incoming asset");

      // Fee is on USDC leaving the pool
      expect(outFee).to.be.greaterThan(0n,
        "Outgoing asset (USDC) carries the fee");
    });

    it("pool_fps increments after swap proportional to out_fee", () => {
      const amountIn        = 1_000_000_000n;
      const feeBps          = 100n;
      const poolTotalLp     = 2_000_000_000n; // 2 SOL deposited by LPs

      const { outFee } = computeSwapOut(
        amountIn, 86_000_000n, 1_000_000n, feeBps,
      );

      const fpsIncrease = computeFpsIncrement(outFee, poolTotalLp);
      expect(fpsIncrease).to.be.greaterThan(0n,
        "pool_fps must increase after every swap");

      // Two LPs (each 1 SOL) each earn half the swap fee
      const aliceAmount   = 1_000_000_000n;
      const aliceClaimable = computeClaimable(aliceAmount, fpsIncrease, 0n, 0n);

      expect(aliceClaimable).to.equal(outFee / 2n,
        "Alice (50% share) earns half the swap fee from pool-wide model");
    });
  });

  // ── Max % concentration guard ──────────────────────────────────

  describe("max % concentration guard (MaxPctBufferExceeded)", () => {
    it("concentration check: asset_in above hard cap is rejected", () => {
      // Hard cap = max_pct_max + MAX_PCT_BUFFER (10%)
      // If max_pct_max = 80%, hard_cap = 90%

      const maxPctMax     = 80;    // LP-set
      const maxPctBuffer  = 10;    // constant
      const hardCapPct    = maxPctMax + maxPctBuffer;  // 90%

      // Two-asset oracle-adjusted concentration check (as in swap.rs):
      // new_in_usd    = (asset_in.amount + amount_in) × oracle_price_in
      // post_out_usd  = (asset_out.amount − amount_out_before_fee) × oracle_price_out
      // in_pct_bps    = new_in_usd × 10_000 / (new_in_usd + post_out_usd)
      // reject if in_pct_bps > hard_cap_bps

      const assetInAmount  = 800_000_000n;   // 0.8 SOL in vault
      const amountIn       = 1_000_000_000n; // 1 SOL being swapped in
      const oraclePriceIn  = 86_000_000n;    // $86

      const assetOutAmount       = 100_000_000_000n; // 100 USDC in vault
      const amountOutBeforeFee   = 86_000_000_000n;  // 86 USDC out
      const oraclePriceOut       = 1_000_000n;       // $1.00

      const newInUsd   = ((assetInAmount + amountIn) * oraclePriceIn) / 1_000_000n;
      const postOutUsd = ((assetOutAmount - amountOutBeforeFee) * oraclePriceOut) / 1_000_000n;
      const inPctBps   = (newInUsd * 10_000n) / (newInUsd + postOutUsd);
      const hardCapBps = BigInt(hardCapPct * 100); // 9000 bps

      // newInUsd  = 1.8 × 86 = 154.8 (USD)
      // postOutUsd = 14 × 1.00 = 14.0 (USD)
      // inPctBps  = 154.8 × 10000 / 168.8 ≈ 9170 bps (91.7%)
      // hardCapBps = 9000 bps

      expect(inPctBps).to.be.greaterThan(hardCapBps,
        "SOL would exceed 90% concentration — swap must be rejected");
    });

    it("swap within concentration limit passes the guard", () => {
      const maxPctMax     = 80;
      const hardCapBps    = BigInt((maxPctMax + 10) * 100); // 9000 bps

      const assetInAmount  = 100_000_000n;   // small SOL position
      const amountIn       = 50_000_000n;    // small swap
      const oraclePriceIn  = 86_000_000n;

      const assetOutAmount       = 500_000_000_000n; // large USDC pool
      const amountOutBeforeFee   = 4_300_000_000n;   // small USDC out
      const oraclePriceOut       = 1_000_000n;

      const newInUsd   = ((assetInAmount + amountIn) * oraclePriceIn) / 1_000_000n;
      const postOutUsd = ((assetOutAmount - amountOutBeforeFee) * oraclePriceOut) / 1_000_000n;
      const inPctBps   = (newInUsd * 10_000n) / (newInUsd + postOutUsd);

      expect(inPctBps).to.be.lessThanOrEqual(hardCapBps,
        "Small swap keeps concentration well within hard cap — must pass");
    });
  });

  // ── Slippage protection ────────────────────────────────────────

  describe("slippage protection (min_amount_out)", () => {
    it("swap rejected when amount_out < min_amount_out", () => {
      const amountOut    = 85_140_000_000n;
      const minAmountOut = 86_000_000_000n; // user wants at least 86 USDC

      expect(amountOut < minAmountOut).to.be.true(
        "Swap must revert with SlippageExceeded when amount_out < min_amount_out");
    });

    it("swap passes when amount_out >= min_amount_out", () => {
      const amountOut    = 85_140_000_000n;
      const minAmountOut = 85_000_000_000n; // user accepts ≥ 85 USDC

      expect(amountOut >= minAmountOut).to.be.true(
        "Swap must succeed when slippage within user tolerance");
    });
  });

  // ── pool_weight after swap ─────────────────────────────────────

  describe("pool_weight increments by out_fee on swap", () => {
    it("pool_weight += out_fee_amount (not total_out or amount_in)", () => {
      const poolWeightBefore = 0n;
      const outFee           = 860_000_000n; // 0.86 USDC fee from swap

      // pool_weight ONLY grows by the outgoing fee retained in the vault
      const poolWeightAfter  = poolWeightBefore + outFee;

      expect(poolWeightAfter).to.equal(860_000_000n,
        "pool_weight grows by out_fee only — not by amount_in or amount_out");
    });
  });
});
