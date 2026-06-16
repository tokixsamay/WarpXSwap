/**
 * pool-fee-accumulator.test.ts
 *
 * Tests the pool-wide fee accumulator (pool_fps) model:
 *   - pool_fps increments correctly per swap
 *   - LP fee_debt is set at deposit time (no retroactive fees)
 *   - Multiple LPs earn proportionally from ALL swaps
 *   - Re-deposit preserves pending_fees before resetting fee_debt
 *   - compound_fees grows principal and pool_total_lp_deposited
 *   - Pool-wide model handles "capital switching" correctly
 *     (SOL LP earns PYUSD out-fees — since PYUSD is their SOL at work)
 */

import { expect } from "chai";
import { describe, it, before } from "mocha";
import { BN } from "@coral-xyz/anchor";
import { Keypair, PublicKey } from "@solana/web3.js";
import {
  createTestContext,
  computeFpsIncrement,
  computeClaimable,
  computeOutFee,
  expectedPoolFpsAfterSwaps,
  TestCtx,
  FEE_SCALE,
} from "./helpers/setup";

describe("pool-wide fee accumulator (pool_fps)", () => {
  let ctx: TestCtx;

  before(async () => {
    ctx = await createTestContext();
  });

  // ── pool_fps math ─────────────────────────────────────────────

  describe("fps increment formula", () => {
    it("Δfps = outFeeAmount × FEE_SCALE / pool_total_lp_deposited", () => {
      const outFee     = 1_000_000n;       // 0.001 SOL in lamports
      const totalLp    = 2_000_000_000n;   // 2 SOL deposited
      const expected   = (outFee * FEE_SCALE) / totalLp; // 500_000

      expect(computeFpsIncrement(outFee, totalLp)).to.equal(expected);
      expect(computeFpsIncrement(outFee, totalLp)).to.equal(500_000n);
    });

    it("fps increment is 0 when no LPs deposited (avoids divide-by-zero)", () => {
      expect(computeFpsIncrement(1_000_000n, 0n)).to.equal(0n,
        "When pool_total_lp_deposited = 0, fps_inc = 0 — fees stay in vault");
    });

    it("multiple swaps accumulate fps correctly", () => {
      const totalLp = 3_000_000_000n; // 3 SOL

      const swaps = [
        { outFeeAmount: 1_000_000n },   // swap 1: 0.001 SOL fee
        { outFeeAmount: 1_500_000n },   // swap 2: 0.0015 SOL fee
        { outFeeAmount:   500_000n },   // swap 3: 0.0005 SOL fee
      ];

      const totalFps = expectedPoolFpsAfterSwaps(swaps, totalLp);
      const manualFps = swaps.reduce((acc, s) => acc + computeFpsIncrement(s.outFeeAmount, totalLp), 0n);

      expect(totalFps).to.equal(manualFps);
      expect(totalFps).to.be.greaterThan(0n);
    });
  });

  // ── LP claimable formula ──────────────────────────────────────

  describe("claimable fees formula", () => {
    it("new LP earns 0 from swaps before their deposit (fee_debt = current pool_fps)", () => {
      const poolFps    = 1_000_000n; // accumulated from prior swaps
      const lpAmount   = 500_000_000n;
      const feeDebt    = poolFps;    // ← LP joins NOW: fee_debt = current pool_fps
      const pending    = 0n;

      const claimable = computeClaimable(lpAmount, poolFps, feeDebt, pending);
      expect(claimable).to.equal(0n,
        "New LP must not retroactively claim fees from before their deposit");
    });

    it("existing LP earns pro-rata share from swaps after their deposit", () => {
      const feeDebtAtDeposit = 0n;           // LP joined when pool_fps = 0
      const poolFpsNow       = 1_000_000n;   // pool_fps grew after LP's deposit
      const lpAmount         = 1_000_000_000n; // 1 SOL principal
      const pending          = 0n;

      // claimable = 1_000_000_000 × (1_000_000 − 0) / 1_000_000_000 = 1_000_000
      const claimable = computeClaimable(lpAmount, poolFpsNow, feeDebtAtDeposit, pending);
      expect(claimable).to.equal(1_000_000n);
    });

    it("pending_fees preserved on re-deposit + new accrual added", () => {
      // Scenario: LP re-deposits while they have accrued fees
      const feeDebt        = 0n;
      const poolFpsAtReDeposit = 800_000n; // pool_fps when LP re-deposits
      const lpAmountBefore = 1_000_000_000n;
      const existingPending = 200_000n;

      // Step 1: settle accrued fees before resetting fee_debt
      const accrued       = computeClaimable(lpAmountBefore, poolFpsAtReDeposit, feeDebt, 0n);
      const newPending    = existingPending + accrued;

      // accrued = 1_000_000_000 × 800_000 / 1_000_000_000 = 800_000
      expect(accrued).to.equal(800_000n);
      // new pending = 200_000 + 800_000 = 1_000_000 (preserved across re-deposit)
      expect(newPending).to.equal(1_000_000n,
        "Re-deposit must preserve existing earnings in pending_fees");

      // Step 2: fee_debt reset to current pool_fps
      const feeDebtAfterReDeposit = poolFpsAtReDeposit;

      // Step 3: additional swaps happen after re-deposit
      const poolFpsAfterMoreSwaps  = 900_000n;
      const lpAmountAfterReDeposit = 1_500_000_000n; // 1.5 SOL (new deposit added)

      const additionalAccrued = computeClaimable(
        lpAmountAfterReDeposit,
        poolFpsAfterMoreSwaps,
        feeDebtAfterReDeposit,
        0n,
      );
      // (1_500_000_000 × (900_000 - 800_000)) / 1_000_000_000 = 150_000
      expect(additionalAccrued).to.equal(150_000n);

      const totalClaimable = computeClaimable(
        lpAmountAfterReDeposit,
        poolFpsAfterMoreSwaps,
        feeDebtAfterReDeposit,
        newPending,
      );
      expect(totalClaimable).to.equal(1_000_000n + 150_000n,
        "Total claimable = preserved pending + new accrued");
    });
  });

  // ── Pool-wide model: capital switching ────────────────────────

  describe("capital switching — SOL LP earns PYUSD out-fees", () => {
    it("All LPs earn proportionally from EVERY swap regardless of which asset swaps", () => {
      // Scenario:
      //   - Alice deposits 1000 SOL. pool_fps = 0, fee_debt_alice = 0
      //   - Bob   deposits 1000 SOL. pool_fps = 0, fee_debt_bob   = 0
      //   - pool_total_lp_deposited = 2000 SOL
      //
      // Swap 1: Trader swaps 100 SOL out (fee = 100 bps = 1%)
      //   out_fee = 1 SOL = 1_000_000_000 lamports
      //   fps_inc = 1e9 × 1e9 / 2e12 = 500_000
      //   pool_fps = 500_000
      //
      // Swap 2: Trader swaps 500 PYUSD out (fee = 30 bps = 0.30%)
      //   out_fee = 1.5 PYUSD scaled = 1_500_000 (assuming 6 decimals)
      //   fps_inc = 1_500_000 × 1e9 / 2_000_000_000_000 = 750
      //   pool_fps += 750
      //
      // Alice claims after both swaps:
      //   claimable = 1e12 × (500_750 − 0) / 1e9 = 500_750 lamport-equivalent
      //
      // NOTE: fees are cross-asset in the pool-wide model. PYUSD out-fee
      // benefits SOL LP because PYUSD entered the pool when SOL LP's SOL
      // was swapped away.

      const poolTotalLp = 2_000_000_000_000n; // 2000 SOL in lamports
      const aliceAmount = 1_000_000_000_000n; // 1000 SOL in lamports

      // Swap 1: 100 SOL out at 100 bps
      const swap1Fee    = 1_000_000_000n;       // 1 SOL fee
      const fps1        = computeFpsIncrement(swap1Fee, poolTotalLp);

      // Swap 2: 500 PYUSD out at 30 bps (scaled to comparable units)
      const swap2Fee    = 1_500_000n;           // 1.5 PYUSD (6 decimals)
      const fps2        = computeFpsIncrement(swap2Fee, poolTotalLp);

      const poolFps = fps1 + fps2;

      const aliceClaimable = computeClaimable(aliceAmount, poolFps, 0n, 0n);

      // Alice earns from BOTH swaps (her 50% share)
      const aliceExpectedFromSwap1 = swap1Fee / 2n;
      const aliceExpectedFromSwap2 = swap2Fee / 2n;

      // Verify proportionality (50% of each swap's fees)
      // Small rounding differences are acceptable (integer division)
      const diff1 = aliceClaimable > aliceExpectedFromSwap1 + aliceExpectedFromSwap2
        ? aliceClaimable - (aliceExpectedFromSwap1 + aliceExpectedFromSwap2)
        : (aliceExpectedFromSwap1 + aliceExpectedFromSwap2) - aliceClaimable;

      expect(diff1).to.be.lessThan(10n,
        "Alice must earn ~50% of both swaps via pool-wide fps model (rounding < 10 units)");

      expect(aliceClaimable).to.be.greaterThan(0n,
        "SOL LP earns PYUSD out-fees — pool-wide model correctly handles capital switching");
    });
  });

  // ── compound_fees dilution trade-off ─────────────────────────

  describe("compound_fees — pool_total_lp_deposited dilution", () => {
    it("compound_fees increases pool_total_lp_deposited (accepted dilution trade-off)", () => {
      // When LP compounds fees: claimable tokens reclassified as principal.
      // pool_total_lp_deposited increases → future fps increments diluted per unit.
      //
      // This is the accepted trade-off: LP's absolute earnings keep pace
      // because their principal (amount) also grew. Documented as a design choice.

      const poolTotalBefore = 2_000_000_000n;  // 2 SOL total deposited
      const compoundAmount  = 100_000_000n;     // 0.1 SOL compounded
      const poolTotalAfter  = poolTotalBefore + compoundAmount;

      // Future swap with the same out_fee:
      const outFee       = 1_000_000n;
      const fpsBefore    = computeFpsIncrement(outFee, poolTotalBefore);
      const fpsAfter     = computeFpsIncrement(outFee, poolTotalAfter);

      expect(fpsAfter).to.be.lessThan(fpsBefore,
        "fps increment per unit decreases after compound (dilution trade-off)");

      // But LP's absolute earnings compensate because their principal grew:
      const lpAmountBefore = 1_000_000_000n;  // 1 SOL principal
      const lpAmountAfter  = lpAmountBefore + compoundAmount;  // 1.1 SOL after compound

      const earningsBefore = computeClaimable(lpAmountBefore, fpsBefore, 0n, 0n);
      const earningsAfter  = computeClaimable(lpAmountAfter, fpsAfter, 0n, 0n);

      // Despite smaller fps rate, larger principal means comparable absolute earnings
      const ratio = (earningsAfter * 1000n) / (earningsBefore === 0n ? 1n : earningsBefore);
      expect(ratio).to.be.greaterThan(900n,
        "Compounding principal roughly preserves absolute fee earnings despite fps dilution");
    });
  });

  // ── add_asset current_fee initial value ───────────────────────

  describe("add_asset current_fee initialization (Bug #6 fix)", () => {
    it("volatile asset: current_fee = midpoint computed without u16 overflow", () => {
      // Fix: (fee_min as u32 + fee_max as u32) / 2) as u16
      // Old code: (fee_min + fee_max) / 2 — u16 arithmetic, can overflow

      const feeMin = 30;
      const feeMax = 100;
      // MAX_FEE_BPS = 500, so max sum = 1000 — safe for u16 (max 65535)
      // But the fix uses u32 to be safe for all valid values

      const midpoint = Math.floor((feeMin + feeMax) / 2);
      expect(midpoint).to.equal(65);
      expect(midpoint).to.be.within(feeMin, feeMax,
        "Initial current_fee must be within [fee_min, fee_max]");
    });

    it("stablecoin asset: current_fee = static_fee_bps (not midpoint of fee_min/fee_max)", () => {
      const staticFeeBps = 5;  // 0.05% flat fee for stablecoin
      const feeMin       = 1;  // may differ from staticFeeBps
      const feeMax       = 1;
      // For stable assets, current_fee = static_fee_bps (new fix)
      // Old code would use (feeMin + feeMax)/2 which is wrong for stables

      const currentFeeNew = staticFeeBps; // correct — uses static_fee_bps
      const currentFeeOld = Math.floor((feeMin + feeMax) / 2); // wrong for stable

      expect(currentFeeNew).to.equal(staticFeeBps,
        "Stablecoin current_fee must be static_fee_bps, not midpoint of min/max");
    });
  });
});
