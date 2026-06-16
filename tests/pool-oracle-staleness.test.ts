/**
 * pool-oracle-staleness.test.ts
 *
 * Integration tests for Bug #3 — oracle staleness rejection.
 *
 * Problem (pre-fix):
 *   swap.rs read oracle_price from AssetAccount but never checked how old
 *   the price was.  If InfoPool's crank stopped running, swaps would continue
 *   to execute at a potentially hours-old price — a critical MEV/arbitrage
 *   attack surface.
 *
 * Fix (post-fix):
 *   AssetAccount gains a new field: oracle_price_slot — the slot at which
 *   InfoPool last pushed a price via push_oracle_price CPI.
 *   Before executing any swap, swap.rs (Step 4) checks BOTH asset_in and
 *   asset_out:
 *     oracle_price_slot > 0                              // price was ever pushed
 *     && current_slot − oracle_price_slot                // age
 *          <= MAX_ORACLE_STALENESS_SLOTS (150)           // ≈ 60 s at 400 ms/slot
 *   Violation → PoolError::OraclePriceStale
 *
 * Test scenarios:
 *   1.  oracle_price_slot = 0 → stale (never pushed)
 *   2.  fresh push (age = 0) → fresh
 *   3.  age = 149 → within window → fresh
 *   4.  age = 150 → exactly at boundary → fresh
 *   5.  age = 151 → one slot over boundary → stale
 *   6.  age = 1000 → very stale → stale
 *   7.  MAX_ORACLE_STALENESS_SLOTS = 150 constant assertion
 *   8.  Both asset_in and asset_out must be fresh for swap to proceed
 *   9.  Stale asset_in alone rejects swap
 *   10. Stale asset_out alone rejects swap
 *   11. Advancing simulated clock by 151 slots turns fresh price stale
 *   12. Re-pushing oracle after staleness restores freshness
 *   13. isOracleFresh is correct at every boundary value
 *   14. Boundary table: slots [0,1,149,150,151,1000] — expected results
 */

import { expect } from "chai";
import { describe, it, before } from "mocha";
import {
  createTestContext,
  MAX_ORACLE_STALENESS_SLOTS,
  isOracleFresh,
  TestCtx,
} from "./helpers/setup";

// ── Boundary constant ─────────────────────────────────────────
const STALENESS_WINDOW = MAX_ORACLE_STALENESS_SLOTS; // 150n

// ── Simulate swap validation (mirrors swap.rs Step 4) ─────────
// Returns true if swap can proceed; false if OraclePriceStale would fire.
function canSwap(
  assetInOracleSlot:  bigint,
  assetOutOracleSlot: bigint,
  currentSlot:        bigint,
): boolean {
  return isOracleFresh(assetInOracleSlot, currentSlot)
      && isOracleFresh(assetOutOracleSlot, currentSlot);
}

describe("Bug #3 — oracle staleness rejection", () => {
  let ctx: TestCtx;

  before(async () => {
    ctx = await createTestContext();
  });

  // ── 1. Constant sanity check ───────────────────────────────────

  describe("MAX_ORACLE_STALENESS_SLOTS constant", () => {
    it("MAX_ORACLE_STALENESS_SLOTS = 150 (≈ 60 seconds at 400 ms/slot)", () => {
      expect(STALENESS_WINDOW).to.equal(150n,
        "Constant must match Rust source — any change is a protocol parameter change");
    });

    it("150 slots × 400 ms/slot = 60 000 ms = 60 seconds (documented window)", () => {
      const slotsToMs = Number(STALENESS_WINDOW) * 400;
      expect(slotsToMs).to.equal(60_000,
        "Staleness window is ~60 seconds — confirms the documented invariant");
    });
  });

  // ── 2. isOracleFresh — boundary table ────────────────────────

  describe("isOracleFresh — boundary values", () => {
    const CURRENT_SLOT = 1000n;

    it("oracle_price_slot = 0 → stale (price was never pushed)", () => {
      expect(isOracleFresh(0n, CURRENT_SLOT)).to.be.false(
        "Slot 0 means oracle was never initialised — must be stale");
    });

    it("age = 0 (just pushed) → fresh", () => {
      const priceSlot = CURRENT_SLOT; // pushed this exact slot
      expect(isOracleFresh(priceSlot, CURRENT_SLOT)).to.be.true(
        "Age 0 — just pushed — must be fresh");
    });

    it("age = 1 → fresh", () => {
      const priceSlot = CURRENT_SLOT - 1n;
      expect(isOracleFresh(priceSlot, CURRENT_SLOT)).to.be.true(
        "Age 1 slot is well within the 150-slot window");
    });

    it("age = 149 → fresh (one slot before boundary)", () => {
      const priceSlot = CURRENT_SLOT - 149n;
      expect(isOracleFresh(priceSlot, CURRENT_SLOT)).to.be.true(
        "Age 149 is inside the staleness window");
    });

    it("age = 150 → fresh (exactly at boundary, inclusive)", () => {
      // Rust: current_slot - oracle_price_slot <= MAX_ORACLE_STALENESS_SLOTS
      // The check is <=, so age 150 is still valid.
      const priceSlot = CURRENT_SLOT - 150n;
      expect(isOracleFresh(priceSlot, CURRENT_SLOT)).to.be.true(
        "Age exactly 150 is at the inclusive boundary — must be fresh");
    });

    it("age = 151 → stale (one slot over the boundary)", () => {
      const priceSlot = CURRENT_SLOT - 151n;
      expect(isOracleFresh(priceSlot, CURRENT_SLOT)).to.be.false(
        "Age 151 exceeds MAX_ORACLE_STALENESS_SLOTS — must be stale");
    });

    it("age = 1 000 → stale (crank has been down a long time)", () => {
      const priceSlot = CURRENT_SLOT - 1_000n;
      expect(isOracleFresh(priceSlot, CURRENT_SLOT)).to.be.false(
        "Age 1000 slots — crank has been down for minutes — must be stale");
    });
  });

  // ── 3. Full boundary table ────────────────────────────────────

  describe("boundary table — all critical ages", () => {
    it("covers [never-pushed, 0, 1, 149, 150, 151, 1000] correctly", () => {
      const CURRENT = 1000n;

      const table: Array<[bigint | "never", boolean, string]> = [
        ["never",         false, "never pushed"],
        [CURRENT - 0n,    true,  "age 0 (just pushed)"],
        [CURRENT - 1n,    true,  "age 1"],
        [CURRENT - 149n,  true,  "age 149 (one before boundary)"],
        [CURRENT - 150n,  true,  "age 150 (at boundary, inclusive)"],
        [CURRENT - 151n,  false, "age 151 (one over boundary)"],
        [CURRENT - 1000n, false, "age 1000 (crank down for ~7 min)"],
      ];

      for (const [priceSlotOrNever, expectedFresh, label] of table) {
        const priceSlot = priceSlotOrNever === "never" ? 0n : priceSlotOrNever;
        const fresh     = isOracleFresh(priceSlot, CURRENT);
        expect(fresh).to.equal(expectedFresh, `${label} — isOracleFresh should be ${expectedFresh}`);
      }
    });
  });

  // ── 4. canSwap — both assets must be fresh ────────────────────

  describe("canSwap — asset_in AND asset_out both checked", () => {
    const SLOT = 1000n;
    const FRESH_SLOT = SLOT - 50n;  // age 50 → fresh
    const STALE_SLOT = SLOT - 200n; // age 200 → stale

    it("both assets fresh → swap proceeds", () => {
      expect(canSwap(FRESH_SLOT, FRESH_SLOT, SLOT)).to.be.true(
        "Both oracles fresh — swap must proceed");
    });

    it("asset_in stale, asset_out fresh → swap rejected", () => {
      expect(canSwap(STALE_SLOT, FRESH_SLOT, SLOT)).to.be.false(
        "Stale asset_in oracle — OraclePriceStale must fire even if asset_out is fresh");
    });

    it("asset_in fresh, asset_out stale → swap rejected", () => {
      expect(canSwap(FRESH_SLOT, STALE_SLOT, SLOT)).to.be.false(
        "Stale asset_out oracle — OraclePriceStale must fire even if asset_in is fresh");
    });

    it("both assets stale → swap rejected", () => {
      expect(canSwap(STALE_SLOT, STALE_SLOT, SLOT)).to.be.false(
        "Both oracles stale — swap must be rejected");
    });

    it("asset_in never pushed (slot=0), asset_out fresh → rejected", () => {
      expect(canSwap(0n, FRESH_SLOT, SLOT)).to.be.false(
        "oracle_price_slot = 0 (never pushed) is treated as stale");
    });

    it("asset_in fresh, asset_out never pushed (slot=0) → rejected", () => {
      expect(canSwap(FRESH_SLOT, 0n, SLOT)).to.be.false(
        "oracle_price_slot = 0 on asset_out — swap rejected");
    });
  });

  // ── 5. Simulated clock advance ────────────────────────────────

  describe("simulated slot-clock advances (bankrun warpSlot equivalent)", () => {
    it("advancing by 150 slots keeps the price fresh (at boundary)", () => {
      const pushSlot    = 1000n;
      const afterSlot   = pushSlot + 150n; // exactly at boundary

      expect(isOracleFresh(pushSlot, afterSlot)).to.be.true(
        "After advancing exactly 150 slots the price is still fresh (inclusive boundary)");
    });

    it("advancing by 151 slots makes the price stale (one over boundary)", () => {
      const pushSlot    = 1000n;
      const afterSlot   = pushSlot + 151n;

      expect(isOracleFresh(pushSlot, afterSlot)).to.be.false(
        "Advancing 151 slots past the push slot crosses the staleness boundary");
    });

    it("re-pushing oracle after staleness restores freshness", () => {
      const firstPushSlot = 1000n;
      const staleSlot     = firstPushSlot + 500n; // crank was down for 500 slots

      // Price is stale at this point
      expect(isOracleFresh(firstPushSlot, staleSlot)).to.be.false(
        "Precondition: price is stale before re-push");

      // InfoPool crank fires again at slot 1500 — new price pushed
      const rePushSlot = staleSlot; // pushed at current slot
      expect(isOracleFresh(rePushSlot, staleSlot)).to.be.true(
        "After re-push at current slot, oracle is fresh again");
    });

    it("freshness degrades slot by slot until boundary", () => {
      const pushSlot = 500n;

      for (let age = 0n; age <= STALENESS_WINDOW; age++) {
        const currentSlot = pushSlot + age;
        expect(isOracleFresh(pushSlot, currentSlot)).to.be.true(
          `Age ${age} should still be fresh (within ${STALENESS_WINDOW}-slot window)`);
      }

      // One slot over the window
      const slotOverWindow = pushSlot + STALENESS_WINDOW + 1n;
      expect(isOracleFresh(pushSlot, slotOverWindow)).to.be.false(
        `Age ${STALENESS_WINDOW + 1n} must be stale (one slot over window)`);
    });
  });

  // ── 6. Interaction with OraclePriceNotSet ─────────────────────

  describe("oracle_price_slot = 0 vs oracle_price = 0 — both guards needed", () => {
    it("oracle_price_slot = 0 catches 'never updated' even if price field is non-zero", () => {
      // oracle_price might be set to a default / stale value in account init,
      // but oracle_price_slot = 0 means InfoPool's crank has NEVER fired.
      // isOracleFresh checks oracle_price_slot first.
      const oraclePriceSlot = 0n;
      const currentSlot     = 100n;

      expect(isOracleFresh(oraclePriceSlot, currentSlot)).to.be.false(
        "oracle_price_slot = 0 must be treated as stale regardless of oracle_price value");
    });

    it("fresh oracle_price_slot ensures the price was pushed recently (complementary to OraclePriceNotSet)", () => {
      // OraclePriceNotSet fires if oracle_price = 0 (never a value pushed).
      // OraclePriceStale fires if oracle_price > 0 but oracle_price_slot is too old.
      // Together they guard all three staleness cases:
      //   (a) Never set   → OraclePriceNotSet
      //   (b) Set but stale  → OraclePriceStale  (this bug's guard)
      //   (c) Set and fresh  → swap proceeds
      const freshSlot   = 950n;
      const currentSlot = 1000n; // age = 50

      expect(isOracleFresh(freshSlot, currentSlot)).to.be.true(
        "Fresh price slot (age=50) passes the staleness guard");
    });
  });

  // ── 7. Regression: pre-fix behaviour ─────────────────────────

  describe("regression — pre-fix swap would ignore staleness", () => {
    it("demonstrates how a 200-slot-old price would have been used pre-fix", () => {
      // Pre-fix: swap.rs had no staleness check.  Only oracle_price != 0 was gated.
      // This test documents what the OLD code accepted (and why it was dangerous).

      const oraclePrice     = 150_000_000n; // $150 SOL — set 200 slots ago
      const oraclePriceSlot = 800n;         // pushed at slot 800
      const currentSlot     = 1000n;        // now at slot 1000 → age 200

      // Pre-fix: only checked oracle_price > 0 (OraclePriceNotSet guard)
      const preFix_wouldAccept = oraclePrice > 0n;

      // Post-fix: also checks staleness
      const postFix_isStale = !isOracleFresh(oraclePriceSlot, currentSlot);

      expect(preFix_wouldAccept).to.be.true(
        "Pre-fix: swap executed at 200-slot-old price — MEV attack surface");
      expect(postFix_isStale).to.be.true(
        "Post-fix: same situation correctly rejected as stale");
    });
  });
});
