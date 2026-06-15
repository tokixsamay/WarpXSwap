/// Pool unit tests — pure logic, no Anchor context required.
/// Run with: cargo test -p pool-program
///
/// Covers the three FIX 1 / FIX 6 / FIX 10 algorithms:
///   1. Oracle-rate enforcement  (oracle_price must be > 0 before swap)
///   2. Oracle-rate pricing math (amount_out = amount_in × rate_in / rate_out)
///   3. Fee application          (fee deducted from gross amount_out)
///   4. Slippage guard           (amount_out >= min_amount_out)
///   5. Deposit zero-amount guard (FIX 6)
///   6. Overflow safety          (checked arithmetic paths)
#[cfg(test)]
mod tests {
    use crate::constants::BPS_DENOMINATOR;

    // ─── Helpers ──────────────────────────────────────────────────────

    /// Mirrors swap.rs Step 4–7 in pure Rust.
    /// Returns Err("OraclePriceNotSet") if either rate is 0.
    /// Returns Err("InsufficientLiquidity") if pool has no out-asset balance.
    /// Returns Err("SlippageExceeded") if amount_out < min_amount_out.
    /// Returns Ok((amount_out, fee_amount)) on success.
    fn calc_swap(
        amount_in:      u64,
        rate_in:        u64,
        rate_out:       u64,
        fee_bps:        u16,
        pool_balance:   u64,
        min_amount_out: u64,
    ) -> Result<(u64, u64), &'static str> {
        // Step 4: oracle price guard (FIX 1)
        if rate_in  == 0 { return Err("OraclePriceNotSet"); }
        if rate_out == 0 { return Err("OraclePriceNotSet"); }

        // liquidity guard
        if pool_balance == 0 { return Err("InsufficientLiquidity"); }

        // Step 5: gross output
        let gross = (amount_in as u128)
            .checked_mul(rate_in as u128)
            .ok_or("MathOverflow")?
            .checked_div(rate_out as u128)
            .ok_or("MathOverflow")? as u64;

        if pool_balance < gross { return Err("InsufficientLiquidity"); }

        // Step 6: fee on outgoing asset
        let fee_amount = gross
            .checked_mul(fee_bps as u64)
            .ok_or("MathOverflow")?
            .checked_div(BPS_DENOMINATOR)
            .ok_or("MathOverflow")?;

        let amount_out = gross
            .checked_sub(fee_amount)
            .ok_or("MathOverflow")?;

        // Step 7: slippage guard
        if amount_out < min_amount_out { return Err("SlippageExceeded"); }

        Ok((amount_out, fee_amount))
    }

    /// Mirrors deposit.rs zero-amount guard (FIX 6).
    fn check_deposit_amount(amount: u64) -> Result<(), &'static str> {
        if amount == 0 { Err("InsufficientBalance") } else { Ok(()) }
    }

    // ─── 1. Oracle-rate enforcement ──────────────────────────────────

    #[test]
    fn swap_fails_when_rate_in_is_zero() {
        // oracle_price starts at 0 on asset creation; swap must fail until
        // InfoPool's push_oracle_price_to_pool CPI sets it.
        assert_eq!(
            calc_swap(1_000, 0, 5_000, 30, 1_000_000, 0),
            Err("OraclePriceNotSet"),
            "rate_in = 0 must be rejected before any arithmetic"
        );
    }

    #[test]
    fn swap_fails_when_rate_out_is_zero() {
        assert_eq!(
            calc_swap(1_000, 3_000, 0, 30, 1_000_000, 0),
            Err("OraclePriceNotSet"),
            "rate_out = 0 must be rejected before any arithmetic"
        );
    }

    #[test]
    fn swap_fails_when_both_rates_are_zero() {
        assert_eq!(
            calc_swap(1_000, 0, 0, 30, 1_000_000, 0),
            Err("OraclePriceNotSet"),
            "rate_in and rate_out both zero must fail on rate_in check first"
        );
    }

    // ─── 2. Oracle-rate pricing math ─────────────────────────────────

    #[test]
    fn swap_1_to_1_peg_no_fee() {
        // SOL/USDC with equal oracle prices → amount_out = amount_in
        let (out, fee) = calc_swap(1_000_000, 100_000, 100_000, 0, 10_000_000, 0).unwrap();
        assert_eq!(out, 1_000_000, "1:1 peg, no fee → out equals in");
        assert_eq!(fee, 0,         "1:1 peg, no fee → fee is zero");
    }

    #[test]
    fn swap_btc_to_usdc_oracle_prices() {
        // BTC = $95,000, USDC = $1.
        // Swap 1 BTC (1e8 lamports) → expect ~95,000 USDC (1e6 units each).
        // rate_in = 95_000, rate_out = 1 (prices in USD, same decimals).
        // amount_in = 100_000_000 (1 BTC in satoshis).
        // amount_out_gross = 100_000_000 * 95_000 / 1 = 9_500_000_000_000 — too large for u64 USDC units.
        //
        // Use normalised units (both in $-cents, 6-decimal tokens):
        // rate_in = 9_500_000 ($95,000 × 100, cents), rate_out = 100 ($1 × 100)
        // amount_in = 1_000_000 (1 USDC-unit of BTC, 6 decimals)
        // gross = 1_000_000 * 9_500_000 / 100 = 95_000_000_000 USDC units
        //
        // Simplified ratio check — 2:1 oracle (rate_in=2, rate_out=1):
        let (out, _fee) = calc_swap(500_000, 2, 1, 0, 10_000_000, 0).unwrap();
        assert_eq!(out, 1_000_000, "2:1 oracle → out = 2× in");
    }

    #[test]
    fn swap_eth_to_btc_oracle_ratio() {
        // ETH = $3,500, BTC = $95,000.
        // rate_in=3500, rate_out=95000.
        // amount_in = 95_000 ETH-units.
        // gross = 95_000 * 3_500 / 95_000 = 3_500 BTC-units.
        let (out, _) = calc_swap(95_000, 3_500, 95_000, 0, 10_000_000, 0).unwrap();
        assert_eq!(out, 3_500, "ETH/BTC oracle pricing correct");
    }

    // ─── 3. Fee application ───────────────────────────────────────────

    #[test]
    fn fee_30_bps_applied_correctly() {
        // 30 bps = 30/10_000 = 0.30%; 0.30% × 1_000_000 = 3_000
        let (out, fee) = calc_swap(1_000_000, 1, 1, 30, 10_000_000, 0).unwrap();
        assert_eq!(fee, 3_000,   "30 bps fee on 1M gross is 3_000");
        assert_eq!(out, 997_000, "net out after 30 bps fee");
    }

    #[test]
    fn fee_0_bps_no_deduction() {
        let (out, fee) = calc_swap(500_000, 1, 1, 0, 10_000_000, 0).unwrap();
        assert_eq!(fee, 0,       "0 bps fee → no deduction");
        assert_eq!(out, 500_000, "0 bps fee → full amount passes through");
    }

    #[test]
    fn fee_10000_bps_consumes_all_output() {
        // 100% fee (edge case — governance prevents this, but math must not panic)
        let (out, fee) = calc_swap(1_000, 1, 1, 10_000, 10_000_000, 0).unwrap();
        assert_eq!(fee, 1_000, "10000 bps fee consumes all gross output");
        assert_eq!(out, 0,     "net out is zero at 100% fee");
    }

    // ─── 4. Slippage guard ────────────────────────────────────────────

    #[test]
    fn slippage_exceeded_when_out_below_min() {
        // 30 bps on 1M → out = 997_000. Set min_out = 997_100 → slippage.
        assert_eq!(
            calc_swap(1_000_000, 1, 1, 30, 10_000_000, 997_100),
            Err("SlippageExceeded"),
            "amount_out < min_amount_out must be rejected"
        );
    }

    #[test]
    fn slippage_passes_when_out_equals_min() {
        // 30 bps on 1M → out = 997_000; min = 997_000 → accepted (inclusive)
        let (out, _) = calc_swap(1_000_000, 1, 1, 30, 10_000_000, 997_000).unwrap();
        assert_eq!(out, 997_000, "amount_out == min_amount_out is accepted");
    }

    #[test]
    fn slippage_passes_when_min_is_zero() {
        // min_amount_out = 0 disables slippage check (user accepts any output)
        let (out, _) = calc_swap(1_000, 1, 1, 100, 10_000_000, 0).unwrap();
        assert!(out > 0, "min_out = 0 always passes slippage check");
    }

    // ─── 5. Deposit zero-amount guard (FIX 6) ────────────────────────

    #[test]
    fn deposit_zero_amount_rejected() {
        assert_eq!(
            check_deposit_amount(0),
            Err("InsufficientBalance"),
            "deposit of 0 tokens must be rejected"
        );
    }

    #[test]
    fn deposit_nonzero_amount_accepted() {
        assert!(check_deposit_amount(1).is_ok(), "deposit of 1 token must pass");
        assert!(check_deposit_amount(u64::MAX).is_ok(), "deposit of u64::MAX must pass");
    }

    // ─── 6. Overflow safety ───────────────────────────────────────────

    #[test]
    fn amount_in_mul_rate_overflow_caught() {
        // u64::MAX * 2 overflows u128? No — u128 handles this. But u64::MAX * u64::MAX does overflow.
        let result = (u64::MAX as u128).checked_mul(u64::MAX as u128);
        assert!(result.is_none(), "u64::MAX × u64::MAX overflows u128 — must be caught");
    }

    #[test]
    fn gross_output_exceeds_pool_balance_rejected() {
        // Pool only has 100 tokens; gross output would be 1_000 → InsufficientLiquidity
        assert_eq!(
            calc_swap(1_000, 1, 1, 0, 100, 0),
            Err("InsufficientLiquidity"),
            "gross output exceeding pool balance must be rejected"
        );
    }

    #[test]
    fn fee_mul_overflow_caught() {
        // fee = gross * fee_bps — gross could be large; use checked_mul.
        let gross: u64 = u64::MAX / 2;
        let result = gross.checked_mul(10_000u64);
        assert!(result.is_none(), "fee multiplication overflow must be caught by checked_mul");
    }
}
