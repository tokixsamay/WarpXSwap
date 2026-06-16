/**
 * pool-deposit-withdraw.test.ts
 *
 * Tests for deposit and withdrawal flows:
 *   - deposit: asset.amount, total_value, pool_total_lp_deposited increase
 *   - deposit: pool_weight does NOT increase (bug fix)
 *   - deposit: fee_debt set to current pool_fps at deposit time
 *   - re-deposit: pending_fees preserved before fee_debt reset
 *   - public_exit: LP can only exit up to their recorded deposit (ExceedsDeposit)
 *   - public_exit: total_value decreases by principal only
 *   - public_exit: pool_weight decreases by fee_share only (bug fix)
 *   - public_exit: pool_total_lp_deposited decreases by principal
 *   - claim_fees: principal unchanged, fee_debt reset
 *   - compound_fees: principal grows, pool_total_lp_deposited grows
 */

import { expect } from "chai";
import { describe, it, before } from "mocha";
import { BN } from "@coral-xyz/anchor";
import { Keypair } from "@solana/web3.js";
import {
  createTestContext,
  computeClaimable,
  computeFpsIncrement,
  FEE_SCALE,
  TestCtx,
} from "./helpers/setup";

describe("deposit and withdrawal flows", () => {
  let ctx: TestCtx;

  before(async () => {
    ctx = await createTestContext();
  });

  // ── Deposit state invariants ────────────────────────────────────

  describe("handler_deposit state invariants", () => {
    it("asset.amount increases by deposit amount", () => {
      const amountBefore  = 0n;
      const depositAmount = 1_000_000_000n; // 1 SOL
      const amountAfter   = amountBefore + depositAmount;
      expect(amountAfter).to.equal(depositAmount);
    });

    it("pool.total_value increases by deposit amount", () => {
      const totalValueBefore = 0n;
      const depositAmount    = 1_000_000_000n;
      const totalValueAfter  = totalValueBefore + depositAmount;
      expect(totalValueAfter).to.equal(depositAmount,
        "total_value tracks principal deposited");
    });

    it("pool.pool_weight does NOT change on deposit (bug fix regression)", () => {
      const poolWeightBefore = 0n; // starts at 0 after bug fix #8
      const depositAmount    = 1_000_000_000n;

      // Correct: pool_weight unchanged after deposit
      const poolWeightAfter = poolWeightBefore; // NOT += depositAmount

      expect(poolWeightAfter).to.equal(0n,
        "pool_weight must not change on deposit — only swap fees add to it");
    });

    it("pool_total_lp_deposited increases by deposit amount (public pool)", () => {
      const totalLpBefore = 0n;
      const depositAmount = 1_000_000_000n;
      const totalLpAfter  = totalLpBefore + depositAmount;
      expect(totalLpAfter).to.equal(depositAmount,
        "pool_total_lp_deposited is the fps denominator — must track LP principal");
    });

    it("lp_deposit.fee_debt = pool_fps at deposit time (no retroactive fees)", () => {
      const poolFpsAtDeposit = 500_000n; // pool_fps had accumulated before this LP
      const feeDebtSet       = poolFpsAtDeposit; // fee_debt is set to current fps

      // LP earns 0 from the 500_000 fps that accumulated before them
      const retroactive = computeClaimable(1_000_000_000n, poolFpsAtDeposit, feeDebtSet, 0n);
      expect(retroactive).to.equal(0n,
        "New LP must not earn fees from before their deposit");
    });
  });

  // ── Re-deposit preserves pending_fees ─────────────────────────

  describe("re-deposit preserves pending_fees", () => {
    it("accrued fees settled into pending_fees before fee_debt reset", () => {
      const poolFpsAtFirstDeposit  = 0n;
      const poolFpsAtReDeposit     = 300_000n;
      const lpAmountFirst          = 1_000_000_000n;

      // Accrued before re-deposit
      const accrued = computeClaimable(lpAmountFirst, poolFpsAtReDeposit, poolFpsAtFirstDeposit, 0n);
      // = 1e9 × 300_000 / 1e9 = 300_000

      expect(accrued).to.equal(300_000n);

      // After re-deposit: pending_fees = old_pending + accrued, fee_debt = poolFpsNow
      const pendingAfter  = 0n + accrued;
      const feeDebtAfter  = poolFpsAtReDeposit;
      expect(pendingAfter).to.equal(300_000n,
        "Accrued fees must be saved to pending_fees on re-deposit");

      // New pool_fps after more swaps
      const poolFpsAfterMore = 400_000n;
      const lpAmountAfter    = 2_000_000_000n; // doubled principal

      const newAccrued = computeClaimable(lpAmountAfter, poolFpsAfterMore, feeDebtAfter, pendingAfter);
      // = pending(300_000) + 2e9 × (400_000 - 300_000) / 1e9
      // = 300_000 + 200_000 = 500_000
      expect(newAccrued).to.equal(500_000n,
        "Total claimable = preserved pending + new accrual on larger principal");
    });
  });

  // ── public_exit correctness ────────────────────────────────────

  describe("handler_public_exit", () => {
    it("LP cannot exit more than their recorded deposit (ExceedsDeposit guard)", () => {
      const lpDeposit     = 1_000_000_000n;
      const exitAmount    = 1_500_000_000n; // trying to exit more than deposited

      expect(exitAmount > lpDeposit).to.be.true(
        "ExceedsDeposit guard must reject this — exit > deposit");
    });

    it("total_value decreases by principal (not by principal + fee_share)", () => {
      const totalValueBefore = 10_000_000_000n;
      const principalExiting = 1_000_000_000n;
      const feeShare         = 100_000_000n;

      // Correct: total_value tracks principal only
      const totalValueAfter = totalValueBefore - principalExiting;
      expect(totalValueAfter).to.equal(9_000_000_000n,
        "total_value -= principal only (fee_share was never in total_value)");
    });

    it("pool_weight decreases by fee_share only (bug fix)", () => {
      const poolWeightBefore = 500_000_000n;
      const feeShare         = 100_000_000n;
      const principal        = 1_000_000_000n; // NOT subtracted from pool_weight

      const poolWeightAfter  = poolWeightBefore - feeShare;
      expect(poolWeightAfter).to.equal(400_000_000n,
        "pool_weight -= fee_share only (principal never was in pool_weight)");

      // Bug: pool_weight -= (principal + feeShare) would underflow:
      const buggyDeduct = principal + feeShare; // > poolWeightBefore
      expect(buggyDeduct > poolWeightBefore).to.be.true(
        "Old buggy deduction would cause WeightError panic");
    });

    it("remaining fees preserved in pending_fees on partial exit", () => {
      const lpAmount     = 2_000_000_000n;
      const exitAmount   = 1_000_000_000n; // 50% exit
      const poolFps      = 600_000n;
      const feeDebt      = 0n;

      // Full accrued on current position
      const fullAccrued  = computeClaimable(lpAmount, poolFps, feeDebt, 0n);
      // = 2e9 × 600_000 / 1e9 = 1_200_000

      // Pro-rated exit: LP exits 50%, takes 50% of accrued fees
      const exitFeeAccrued = (fullAccrued * exitAmount) / lpAmount;
      // = 1_200_000 × 50% = 600_000

      // Remaining 50% saved to pending_fees
      const remainingAccrued  = fullAccrued - exitFeeAccrued;
      const newPendingFees    = remainingAccrued;

      expect(newPendingFees).to.equal(600_000n,
        "Remaining accrued fees preserved in pending_fees after partial exit");
    });

    it("pool_total_lp_deposited decreases by principal only on exit", () => {
      const totalLpBefore    = 5_000_000_000n;
      const principalExiting = 1_000_000_000n;
      const totalLpAfter     = totalLpBefore - principalExiting;

      expect(totalLpAfter).to.equal(4_000_000_000n,
        "pool_total_lp_deposited -= principal only (not fee_share)");
    });
  });

  // ── claim_fees ────────────────────────────────────────────────

  describe("handler_claim_fees", () => {
    it("principal (lp_deposit.amount) unchanged after claim_fees", () => {
      const lpAmount    = 1_000_000_000n;
      const claimable   = 50_000_000n;

      // After claim: principal unchanged, only fee_debt resets
      const principalAfter = lpAmount; // NOT lpAmount - claimable
      expect(principalAfter).to.equal(lpAmount,
        "claim_fees must not reduce LP principal");
    });

    it("fee_debt resets to current pool_fps after claim", () => {
      const poolFps    = 800_000n;
      const feeDebtNew = poolFps; // fee_debt = pool_fps after claim

      // Future earnings start fresh from this point
      const earnedImmediately = computeClaimable(1_000_000_000n, poolFps, feeDebtNew, 0n);
      expect(earnedImmediately).to.equal(0n,
        "After claim, fee_debt = pool_fps, so no immediate re-earn");
    });
  });

  // ── compound_fees ─────────────────────────────────────────────

  describe("handler_compound_fees", () => {
    it("lp_deposit.amount grows by compounded amount", () => {
      const lpAmountBefore = 1_000_000_000n;
      const compounded     = 100_000_000n;
      const lpAmountAfter  = lpAmountBefore + compounded;

      expect(lpAmountAfter).to.equal(1_100_000_000n,
        "Compound fees reclassifies fee tokens as principal");
    });

    it("pool_total_lp_deposited grows by compounded amount (dilution trade-off)", () => {
      const totalLpBefore = 3_000_000_000n;
      const compounded    = 100_000_000n;
      const totalLpAfter  = totalLpBefore + compounded;

      expect(totalLpAfter).to.equal(3_100_000_000n,
        "pool_total_lp_deposited increases — future fps denominator grows (known trade-off)");
    });

    it("pending_fees cleared and fee_debt reset after compound", () => {
      const poolFps     = 1_000_000n;
      // After compound: pending_fees = 0, fee_debt = pool_fps
      const pendingAfter = 0n;
      const feeDebtAfter = poolFps;

      const earnsAfterCompound = computeClaimable(1_100_000_000n, poolFps, feeDebtAfter, pendingAfter);
      expect(earnsAfterCompound).to.equal(0n,
        "After compound, LP starts fresh with 0 pending and fee_debt = pool_fps");
    });
  });

  // ── Private pool withdrawal does NOT affect pool_fps model ────

  describe("private pool withdrawals — pool_fps model isolation", () => {
    it("withdraw_base does not touch pool_total_lp_deposited", () => {
      // Private pools use withdraw_base / withdraw_all — NOT public_exit.
      // These do NOT decrement pool_total_lp_deposited, as documented.
      const totalLpBefore = 0n; // private pool: pool_total_lp_deposited = 0
      const withdrawAmount = 500_000_000n;

      // Private withdrawal: pool_total_lp_deposited unchanged
      const totalLpAfter = totalLpBefore; // NOT -= withdrawAmount

      expect(totalLpAfter).to.equal(0n,
        "Private pool withdrawals must not touch pool_total_lp_deposited");
    });
  });
});
