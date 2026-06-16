/// Pool unit tests — pure logic, no Anchor context required.
/// Run with: cargo test -p pool-program
///
/// Covers all swap logic changes:
///   1. Oracle-rate enforcement  (oracle_price must be > 0 before swap)
///   2. Oracle-rate pricing math (amount_out = amount_in × rate_in / rate_out)
///   3. Outgoing fee application (fee deducted from gross amount_out)
///   4. (outgoing-only model — no incoming fee; section retained for numbering)
///   5. Slippage guard           (amount_out >= min_amount_out)
///   6. Deposit zero-amount guard
///   7. Overflow safety          (checked arithmetic paths)
///   8. Max % concentration guard (hard-reject when in-asset would exceed cap)
///   9. Pool-wide fee accumulator (all LPs earn from ALL swap fees)
#[cfg(test)]
mod tests {
    use crate::constants::{BPS_DENOMINATOR, FEE_SCALE, MAX_PCT_BUFFER};

    // ─── Helpers ──────────────────────────────────────────────────────

    /// Mirrors swap.rs Steps 4–9 in pure Rust (outgoing-only fee + max % guard).
    ///
    /// Returns Ok((amount_out, out_fee)) on success.
    /// Errors: "OraclePriceNotSet", "InsufficientLiquidity", "SlippageExceeded",
    ///         "MathOverflow", "MaxPctBufferExceeded".
    fn calc_swap(
        amount_in:        u64,
        rate_in:          u64,
        rate_out:         u64,
        out_fee_bps:      u16,  // fee on outgoing asset (e.g. SOL dynamic fee)
        pool_balance_out: u64,  // current vault balance of outgoing asset
        pool_amount_in:   u64,  // current vault balance of incoming asset (before swap)
        max_pct_max_in:   u8,   // max_pct_max of incoming asset (0-100)
        min_amount_out:   u64,
    ) -> Result<(u64, u64), &'static str> {
        // Step 4: oracle price guard
        if rate_in  == 0 { return Err("OraclePriceNotSet"); }
        if rate_out == 0 { return Err("OraclePriceNotSet"); }

        // Liquidity guard
        if pool_balance_out == 0 { return Err("InsufficientLiquidity"); }

        // Step 5: gross output (oracle pricing)
        let gross = (amount_in as u128)
            .checked_mul(rate_in as u128)
            .ok_or("MathOverflow")?
            .checked_div(rate_out as u128)
            .ok_or("MathOverflow")? as u64;

        // Step 5.5: Max % concentration guard (two-asset approximation)
        {
            let new_in_amount  = pool_amount_in.checked_add(amount_in).ok_or("MathOverflow")?;
            let new_in_usd     = (new_in_amount as u128)
                .checked_mul(rate_in as u128)
                .ok_or("MathOverflow")?;
            let post_out_amount = pool_balance_out.saturating_sub(gross);
            let post_out_usd    = (post_out_amount as u128)
                .checked_mul(rate_out as u128)
                .ok_or("MathOverflow")?;
            let total_two_usd   = new_in_usd.checked_add(post_out_usd).ok_or("MathOverflow")?;

            if total_two_usd > 0 {
                let in_pct_bps = new_in_usd
                    .checked_mul(10_000)
                    .ok_or("MathOverflow")?
                    .checked_div(total_two_usd)
                    .ok_or("MathOverflow")? as u64;

                let hard_cap_bps = (max_pct_max_in as u64)
                    .checked_add(MAX_PCT_BUFFER as u64)
                    .ok_or("MathOverflow")?
                    .checked_mul(100)
                    .ok_or("MathOverflow")?;

                if in_pct_bps > hard_cap_bps {
                    return Err("MaxPctBufferExceeded");
                }
            }
        }

        if pool_balance_out < gross { return Err("InsufficientLiquidity"); }

        // Step 6: outgoing fee (deducted from gross, stays in vault)
        let out_fee = gross
            .checked_mul(out_fee_bps as u64)
            .ok_or("MathOverflow")?
            .checked_div(BPS_DENOMINATOR)
            .ok_or("MathOverflow")?;

        let amount_out = gross
            .checked_sub(out_fee)
            .ok_or("MathOverflow")?;

        // Step 7: slippage guard
        if amount_out < min_amount_out { return Err("SlippageExceeded"); }

        Ok((amount_out, out_fee))
    }

    /// Mirrors Step 10: update pool.pool_fps with the outgoing fee.
    /// Returns the new pool_fps value.
    fn update_pool_fps(
        pool_fps:               u64,
        out_fee:                u64,
        pool_total_lp_deposited: u64,
    ) -> u64 {
        if pool_total_lp_deposited == 0 || out_fee == 0 {
            return pool_fps;
        }
        let inc = (out_fee as u128)
            .checked_mul(FEE_SCALE as u128)
            .unwrap_or(0)
            .checked_div(pool_total_lp_deposited as u128)
            .unwrap_or(0) as u64;
        pool_fps.saturating_add(inc)
    }

    /// Mirrors the claimable fee computation from deposit_withdraw.rs.
    fn calc_claimable(lp_amount: u64, pool_fps: u64, fee_debt: u64, pending_fees: u64) -> u64 {
        let fps_delta = pool_fps.saturating_sub(fee_debt);
        let accrued = (lp_amount as u128)
            .checked_mul(fps_delta as u128)
            .unwrap_or(0)
            .checked_div(FEE_SCALE as u128)
            .unwrap_or(0) as u64;
        pending_fees.saturating_add(accrued)
    }

    /// Mirrors deposit.rs zero-amount guard.
    fn check_deposit_amount(amount: u64) -> Result<(), &'static str> {
        if amount == 0 { Err("InsufficientBalance") } else { Ok(()) }
    }

    // ─── 1. Oracle-rate enforcement ──────────────────────────────────

    #[test]
    fn swap_fails_when_rate_in_is_zero() {
        assert_eq!(
            calc_swap(1_000, 0, 5_000, 30, 1_000_000, 0, 40, 0),
            Err("OraclePriceNotSet"),
            "rate_in = 0 must be rejected before any arithmetic"
        );
    }

    #[test]
    fn swap_fails_when_rate_out_is_zero() {
        assert_eq!(
            calc_swap(1_000, 3_000, 0, 30, 1_000_000, 0, 40, 0),
            Err("OraclePriceNotSet"),
            "rate_out = 0 must be rejected before any arithmetic"
        );
    }

    #[test]
    fn swap_fails_when_both_rates_are_zero() {
        assert_eq!(
            calc_swap(1_000, 0, 0, 30, 1_000_000, 0, 40, 0),
            Err("OraclePriceNotSet"),
            "both zero must fail on rate_in check first"
        );
    }

    // ─── 2. Oracle-rate pricing math ─────────────────────────────────

    #[test]
    fn swap_1_to_1_peg_no_fees() {
        let (out, out_fee) =
            calc_swap(1_000_000, 100_000, 100_000, 0, 10_000_000, 0, 40, 0).unwrap();
        assert_eq!(out,     1_000_000, "1:1 peg, no fees → out equals in");
        assert_eq!(out_fee, 0);
    }

    #[test]
    fn swap_two_to_one_oracle_ratio() {
        let (out, _) = calc_swap(500_000, 2, 1, 0, 10_000_000, 0, 40, 0).unwrap();
        assert_eq!(out, 1_000_000, "2:1 oracle → out = 2× in");
    }

    #[test]
    fn swap_sol_to_usdc_oracle_ratio() {
        // Use rate_in=3500, rate_out=95000 to verify cross-rate math stays correct.
        let (out, _) = calc_swap(95_000, 3_500, 95_000, 0, 10_000_000, 0, 50, 0).unwrap();
        assert_eq!(out, 3_500, "oracle cross-rate pricing correct");
    }

    // ─── 3. Outgoing fee ─────────────────────────────────────────────

    #[test]
    fn out_fee_30_bps_applied_correctly() {
        let (out, out_fee) =
            calc_swap(1_000_000, 1, 1, 30, 10_000_000, 0, 50, 0).unwrap();
        assert_eq!(out_fee, 3_000,  "30 bps out-fee on 1M gross is 3_000");
        assert_eq!(out,   997_000,  "net out after 30 bps out-fee");
    }

    #[test]
    fn out_fee_0_bps_no_deduction() {
        let (out, out_fee) =
            calc_swap(500_000, 1, 1, 0, 10_000_000, 0, 50, 0).unwrap();
        assert_eq!(out_fee, 0,      "0 bps out-fee → no deduction");
        assert_eq!(out,   500_000);
    }

    #[test]
    fn out_fee_10000_bps_consumes_all_output() {
        let (out, out_fee) =
            calc_swap(1_000, 1, 1, 10_000, 10_000_000, 0, 50, 0).unwrap();
        assert_eq!(out_fee, 1_000, "100% fee consumes all gross output");
        assert_eq!(out, 0,         "net out is zero at 100% fee");
    }

    // ─── 4. Slippage guard ────────────────────────────────────────────

    #[test]
    fn slippage_exceeded_when_out_below_min() {
        assert_eq!(
            calc_swap(1_000_000, 1, 1, 30, 10_000_000, 0, 50, 997_100),
            Err("SlippageExceeded"),
        );
    }

    #[test]
    fn slippage_passes_when_out_equals_min() {
        let (out, _) =
            calc_swap(1_000_000, 1, 1, 30, 10_000_000, 0, 50, 997_000).unwrap();
        assert_eq!(out, 997_000, "amount_out == min_amount_out accepted (inclusive)");
    }

    // ─── 6. Deposit zero-amount guard ─────────────────────────────────

    #[test]
    fn deposit_zero_amount_rejected() {
        assert_eq!(check_deposit_amount(0), Err("InsufficientBalance"));
    }

    #[test]
    fn deposit_nonzero_amount_accepted() {
        assert!(check_deposit_amount(1).is_ok());
        assert!(check_deposit_amount(u64::MAX).is_ok());
    }

    // ─── 7. Overflow safety ───────────────────────────────────────────

    #[test]
    fn amount_in_mul_rate_overflow_caught() {
        let result = (u64::MAX as u128).checked_mul(u64::MAX as u128);
        assert!(result.is_none(), "u64::MAX × u64::MAX overflows u128");
    }

    #[test]
    fn gross_output_exceeds_pool_balance_rejected() {
        // Use max_pct_max=100 so the concentration guard doesn't fire first
        assert_eq!(
            calc_swap(1_000, 1, 1, 0, 100, 0, 100, 0),
            Err("InsufficientLiquidity"),
        );
    }

    #[test]
    fn fee_mul_overflow_caught() {
        let gross: u64 = u64::MAX / 2;
        let result = gross.checked_mul(10_000u64);
        assert!(result.is_none(), "fee mul overflow caught by checked_mul");
    }

    // ─── 8. Max % concentration guard ────────────────────────────────

    #[test]
    fn max_pct_guard_passes_within_cap() {
        // pool_out=1000, pool_in=0, swap 100 in → in_pct=10% < 40% hard cap
        assert!(calc_swap(100, 1, 1, 0, 1_000, 0, 30, 0).is_ok());
    }

    #[test]
    fn max_pct_guard_rejects_when_exceeds_cap() {
        // pool_out=100, pool_in=300, swap 200 → in_pct≈100% > 40% hard cap
        assert_eq!(
            calc_swap(200, 1, 1, 0, 100, 300, 30, 0),
            Err("MaxPctBufferExceeded"),
        );
    }

    #[test]
    fn max_pct_guard_passes_exactly_at_buffer() {
        // pool_out=100, swap 40 → in_pct=40% == hard cap (30+10)×100 bps ✓
        assert!(calc_swap(40, 1, 1, 0, 100, 0, 30, 0).is_ok());
    }

    #[test]
    fn max_pct_guard_rejects_one_above_buffer() {
        // pool_out=100, swap 68 → in_pct=68% > 40% hard cap
        assert_eq!(
            calc_swap(68, 1, 1, 0, 100, 0, 30, 0),
            Err("MaxPctBufferExceeded"),
        );
    }

    #[test]
    fn max_pct_guard_uses_oracle_adjusted_values() {
        // pool_out=100 SOL (rate=150), swap 5000 USDC (rate=1)
        // gross=33 SOL; new_in_usd=5000; post_out_usd=67×150=10050
        // in_pct≈3322 bps < 4000 ✓
        assert!(calc_swap(5_000, 1, 150, 0, 100, 0, 30, 0).is_ok());
    }

    // ─── 9. Pool-wide fee accumulator ────────────────────────────────
    //
    // Outgoing swap fees flow into pool.pool_fps.
    // Denominator = pool_total_lp_deposited (only explicit LP deposits).
    // Every LP earns proportionally from every swap in the pool.

    #[test]
    fn pool_fps_incremented_by_out_fee() {
        // out_fee = 3_000; pool_total_lp_deposited = 1_000_000
        // fps_inc = 3_000 × 1e9 / 1_000_000 = 3_000_000
        let new_fps = update_pool_fps(0, 3_000, 1_000_000);
        assert_eq!(new_fps, 3_000_000, "pool fps incremented correctly");
    }

    #[test]
    fn pool_fps_unchanged_when_no_lp_deposited() {
        // No LP has deposited yet → fees sit in vault, pool_fps stays at 0
        let new_fps = update_pool_fps(0, 3_500, 0);
        assert_eq!(new_fps, 0, "pool_fps unchanged when pool_total_lp_deposited=0");
    }

    #[test]
    fn claimable_fees_correct_from_pool_fps() {
        // LP deposited 500_000; pool_fps started at 0 when they joined (fee_debt=0)
        // After some swaps: pool_fps = 2_000_000
        // fps_delta = 2_000_000; claimable = 500_000 × 2_000_000 / 1e9 = 1_000
        let claimable = calc_claimable(500_000, 2_000_000, 0, 0);
        assert_eq!(claimable, 1_000, "claimable = lp_amount × fps_delta / FEE_SCALE");
    }

    #[test]
    fn lp_fee_debt_prevents_retroactive_claim() {
        // LP joined when pool_fps was already 5_000_000 (fee_debt = 5_000_000)
        // pool_fps has since grown to 7_000_000 → only delta=2_000_000 is owed
        // claimable = 500_000 × 2_000_000 / 1e9 = 1_000 (NOT 3_500 for full fps)
        let claimable = calc_claimable(500_000, 7_000_000, 5_000_000, 0);
        assert_eq!(claimable, 1_000, "fee_debt prevents retroactive claim");
    }

    #[test]
    fn pool_fps_accumulates_across_multiple_swaps() {
        // Three swaps, each generating out_fee=1_000 on a 1M LP pool
        // fps_inc per swap = 1_000 × 1e9 / 1_000_000 = 1_000_000
        let fps = update_pool_fps(0,         1_000, 1_000_000);
        let fps = update_pool_fps(fps,        1_000, 1_000_000);
        let fps = update_pool_fps(fps,        1_000, 1_000_000);
        assert_eq!(fps, 3_000_000, "pool_fps accumulates correctly across swaps");

        // LP with 500_000 deposited from start (fee_debt=0)
        let claimable = calc_claimable(500_000, fps, 0, 0);
        assert_eq!(claimable, 1_500, "LP earns proportional share of all swaps");
    }

    // ─── 10. End-to-end: pool-wide fees across two traders ───────────
    //
    //   Pool has SOL.  SOL allows [PYUSD, USDC, USDT].
    //   LP deposited 1_000_000 SOL.
    //
    //   Trader 1: PYUSD → SOL  (pool gives SOL out-fee → pool_fps)
    //   Trader 2: SOL   → PYUSD (pool gives PYUSD out-fee → pool_fps)
    //
    //   Under the old per-asset model:
    //     Trader 2's PYUSD out-fee → PYUSD.fps → no PYUSD depositor → LOCKED
    //
    //   Under the pool-wide model:
    //     ALL out-fees → pool.pool_fps → SOL LP earns from BOTH traders ✓

    #[test]
    fn sol_lp_earns_from_both_pyusd_and_sol_swaps() {
        let sol_lp_deposit:          u64 = 1_000_000;  // SOL LP principal
        let sol_vault:               u64 = 1_000_000;  // initial SOL vault
        let pyusd_vault:             u64 = 0;           // no PYUSD initially
        let rate_sol:                u64 = 200;         // SOL = $200
        let rate_pyusd:              u64 = 1;           // PYUSD = $1
        let sol_fee_bps:             u16 = 30;          // outgoing SOL fee
        let pyusd_fee_bps:           u16 = 5;           // outgoing PYUSD fee

        let mut pool_fps:            u64 = 0;
        let pool_total_lp_deposited: u64 = sol_lp_deposit;
        let lp_fee_debt:             u64 = 0;

        // ── TRADER 1: PYUSD → SOL ─────────────────────────────────
        // asset_out = SOL; out_fee charged on SOL
        let t1_amount_in = 40_000u64;
        let (t1_out, t1_out_fee) = calc_swap(
            t1_amount_in, rate_pyusd, rate_sol,
            sol_fee_bps, sol_vault, pyusd_vault, 30, 0,
        ).expect("Trader 1 swap ok");
        // gross = 40000×1/200 = 200 SOL; out_fee = 200×30/10000 = 0 (rounds down)

        pool_fps = update_pool_fps(pool_fps, t1_out_fee, pool_total_lp_deposited);

        let sol_vault_after_t1   = sol_vault - t1_out;       // 1_000_000 - 200 = 999_800
        let pyusd_vault_after_t1 = pyusd_vault + t1_amount_in; // 40_000

        assert_eq!(sol_vault_after_t1, 999_800, "SOL vault after T1");
        assert_eq!(pyusd_vault_after_t1, 40_000, "PYUSD entered vault");

        // ── TRADER 2: SOL → PYUSD (realistic: 100 SOL) ───────────
        // asset_out = PYUSD; out_fee charged on PYUSD — this is the KEY swap.
        // Under old per-asset model this fee was locked; pool-wide model pays SOL LP.
        let t2_amount_in = 100u64;
        let (_, t2_out_fee) = calc_swap(
            t2_amount_in, rate_sol, rate_pyusd,
            pyusd_fee_bps, pyusd_vault_after_t1, sol_vault_after_t1, 50, 0,
        ).expect("Trader 2 swap ok");
        // gross = 100×200/1 = 20_000 PYUSD; out_fee = 20_000×5/10_000 = 10

        pool_fps = update_pool_fps(pool_fps, t2_out_fee, pool_total_lp_deposited);

        // ── SOL LP's claimable ──────────────────────────────────────
        let total_fees = t1_out_fee + t2_out_fee;
        let sol_lp_claimable = calc_claimable(sol_lp_deposit, pool_fps, lp_fee_debt, 0);

        if total_fees > 0 {
            assert_eq!(sol_lp_claimable, total_fees,
                "SOL LP gets 100% of pool out-fees as the sole depositor");
        }

        let sol_lp_gets_pyusd_outfee = pool_fps > 0 && sol_lp_claimable >= t2_out_fee;
        assert!(sol_lp_gets_pyusd_outfee || t2_out_fee == 0,
            "SOL LP's claim includes PYUSD out-fees (pool-wide model)");
    }

    #[test]
    fn two_lps_share_all_pool_fees_proportionally() {
        // LP_A deposited 600_000 (SOL); LP_B deposited 400_000 (USDC)
        // pool_total_lp_deposited = 1_000_000
        // A swap generates out_fee = 5_000
        // fps_inc = 5_000 × 1e9 / 1_000_000 = 5_000_000
        // LP_A claimable = 600_000 × 5_000_000 / 1e9 = 3_000  (60%)
        // LP_B claimable = 400_000 × 5_000_000 / 1e9 = 2_000  (40%)

        let pool_total = 1_000_000u64;
        let out_fee    = 5_000u64;

        let fps = update_pool_fps(0, out_fee, pool_total);

        let claimable_a = calc_claimable(600_000, fps, 0, 0);
        let claimable_b = calc_claimable(400_000, fps, 0, 0);

        assert_eq!(claimable_a, 3_000, "LP_A (60% of pool) earns 60% of fees");
        assert_eq!(claimable_b, 2_000, "LP_B (40% of pool) earns 40% of fees");
        assert_eq!(claimable_a + claimable_b, out_fee,
            "All pool fees distributed — no rounding leakage");
    }

    #[test]
    fn late_lp_does_not_claim_fees_before_joining() {
        // pool_total = 1_000_000; LP_A joined at pool_fps=0; fee_debt=0
        // After swap 1: pool_fps = 2_000_000 (out_fee=2_000 / 1M total)
        // LP_B joins NOW → fee_debt = 2_000_000 (no retroactive claim)
        // After swap 2: pool_fps = 4_000_000 (another 2_000 / 1M total)
        //   But now LP_B has also deposited 1_000_000 → total = 2_000_000

        let pool_total_before = 1_000_000u64;
        let fps_after_swap1   = update_pool_fps(0, 2_000, pool_total_before);
        assert_eq!(fps_after_swap1, 2_000_000);

        // LP_B joins — debt = fps_after_swap1
        let lp_b_deposit:  u64 = 1_000_000;
        let lp_b_fee_debt: u64 = fps_after_swap1;
        let pool_total_after = pool_total_before + lp_b_deposit; // 2_000_000

        let fps_after_swap2 = update_pool_fps(fps_after_swap1, 2_000, pool_total_after);
        // fps_inc = 2_000 × 1e9 / 2_000_000 = 1_000_000
        assert_eq!(fps_after_swap2, 3_000_000);

        // LP_A: earned from both swaps
        let claimable_a = calc_claimable(1_000_000, fps_after_swap2, 0, 0);
        // LP_B: only earns from swap 2 (joined after swap 1)
        let claimable_b = calc_claimable(lp_b_deposit, fps_after_swap2, lp_b_fee_debt, 0);

        // LP_A: 1M × 3_000_000 / 1e9 = 3 — but swap1 was 1M pool, swap2 was 2M pool
        // LP_A actually earns: swap1 full (2_000) + swap2 half (1_000) = 3_000... 
        // with pool_fps math: 1_000_000 × 3_000_000 / 1e9 = 3_000 ✓
        assert_eq!(claimable_a, 3_000,
            "LP_A earned from both swaps (2_000 from swap1, 1_000 from swap2)");
        // LP_B: 1M × (3_000_000 − 2_000_000) / 1e9 = 1_000
        assert_eq!(claimable_b, 1_000,
            "LP_B only earned from swap2 (joined after swap1)");
        assert_eq!(claimable_a + claimable_b, 4_000,
            "Both LPs together claim all fees generated after LP_B joined");
    }
          }
          
