use crate::constants::*;
use crate::state::ThresholdState;

/// Calculate current fee using a V-shape (smile) curve.
///
/// Fee is highest at the base price (neutral) and slides toward fee_min
/// as price deviates in EITHER direction from base.
///
/// ┌──────────────────────────────────────────────────────┐
/// │  fee_max ──●─────────────────────────────────────    │
/// │             \                                   /    │
/// │              \                                 /     │
/// │               \                               /      │
/// │  fee_min ──────●─────────────────────────────●──     │
/// │             base-dn          base         base+up    │
/// └──────────────────────────────────────────────────────┘
///
/// Rationale: large deviations (either way) attract arbitrageurs who
/// restore the price → lower fees incentivise this → less IL for LPs.
/// Near base the pool is in equilibrium → max fees capture swap revenue.
/// Extreme breaches are handled separately by the inflow-block mechanism.
///
/// Formula (both directions):
///   strength    = min(|deviation_bps|, threshold_bps) × FEE_SENSITIVITY
///                 ─────────────────────────────────────────────────────
///                              threshold_bps × 100
///   fee_reduction = fee_range × strength / 100
///   fee           = fee_max − fee_reduction
pub fn calculate_fee(
    current_price:  i64,
    current_base:   i64,
    threshold_up:   u16,
    threshold_down: u16,
    fee_min:        u16,
    fee_max:        u16,
) -> u16 {
    if current_base == 0 {
        return fee_max;
    }

    // Signed deviation in basis points
    let growth_bps = ((current_price - current_base) as i128)
        .checked_mul(BPS_DENOMINATOR as i128)
        .unwrap_or(0)
        / current_base as i128;

    let fee_range = (fee_max - fee_min) as i128;

    let (deviation_bps, threshold_bps): (i128, i128) = if growth_bps > 0 {
        (growth_bps, threshold_up as i128)
    } else if growth_bps < 0 {
        (-growth_bps, threshold_down as i128)
    } else {
        // Exactly at base — maximum fee
        return fee_max;
    };

    if threshold_bps == 0 {
        return fee_max;
    }

    // Strength: 0 (at base) → 1 (at/beyond threshold), scaled by sensitivity
    let strength = deviation_bps
        .min(threshold_bps)
        .checked_mul(FEE_SENSITIVITY as i128)
        .unwrap_or(0)
        / (threshold_bps * 100);

    let fee_reduction = fee_range
        .checked_mul(strength)
        .unwrap_or(0)
        / 100;

    let fee = fee_max as i128 - fee_reduction;
    fee.max(fee_min as i128).min(fee_max as i128) as u16
}

/// Calculate threshold state based on current price vs base
pub fn calculate_threshold_state(
    current_price:  i64,
    current_base:   i64,
    threshold_up:   u16,
    threshold_down: u16,
) -> ThresholdState {
    if current_base == 0 {
        return ThresholdState::Neutral;
    }

    let growth_bps = ((current_price - current_base) as i128)
        .checked_mul(BPS_DENOMINATOR as i128)
        .unwrap_or(0)
        / current_base as i128;

    if growth_bps > 0 {
        let threshold_bps = threshold_up as i128;
        if growth_bps >= threshold_bps {
            ThresholdState::ExceededUp
        } else {
            // Calculate % of threshold reached (0-100)
            let pct = (growth_bps * 100 / threshold_bps) as u8;
            ThresholdState::ApproachingUp(pct)
        }
    } else if growth_bps < 0 {
        let decline_bps   = (-growth_bps) as i128;
        let threshold_bps = threshold_down as i128;
        if decline_bps >= threshold_bps {
            ThresholdState::ExceededDown
        } else {
            let pct = (decline_bps * 100 / threshold_bps) as u8;
            ThresholdState::ApproachingDown(pct)
        }
    } else {
        ThresholdState::Neutral
    }
}

/// Check if threshold exceeded → should block new inflow.
///
/// Blocks whenever the 3-layer Pyth engine confirms a threshold breach
/// (ExceededUp or ExceededDown). The Pool program enforces max_pct_max
/// independently on every deposit, providing the concentration guard.
pub fn should_block_inflow(state: &ThresholdState) -> bool {
    matches!(state, ThresholdState::ExceededUp | ThresholdState::ExceededDown)
}

/// Check TWAP layer confirmation
/// All three timeframes must trend same direction
pub fn check_twap_layer(
    twap_short:  i64,
    twap_medium: i64,
    twap_long:   i64,
    current:     i64,
) -> bool {
    // All trending up: current > short > medium > long
    let all_up = current > twap_short
        && twap_short > twap_medium
        && twap_medium > twap_long;

    // All trending down: current < short < medium < long
    let all_down = current < twap_short
        && twap_short < twap_medium
        && twap_medium < twap_long;

    all_up || all_down
}

/// Check volume layer confirmation
/// Volume must be consistently increasing
pub fn check_volume_layer(
    volume_current: u64,
    volume_prev:    u64,
) -> bool {
    if volume_prev == 0 {
        return false;
    }
    // Volume must be at least 10% higher than previous period
    let min_increase = volume_prev
        .saturating_add(volume_prev / 10);
    volume_current >= min_increase
}

/// Check confidence interval layer
/// Narrow interval = publishers agree = genuine price
pub fn check_confidence_layer(
    price:      i64,
    confidence: u64,
) -> bool {
    if price <= 0 {
        return false;
    }
    // Confidence must be < CONFIDENCE_RATIO_BPS % of price
    let max_confidence = (price.unsigned_abs())
        .checked_mul(CONFIDENCE_RATIO_BPS)
        .unwrap_or(u64::MAX)
        / BPS_DENOMINATOR;

    confidence <= max_confidence
}

/// Time-weighted Exponential Moving Average update.
///
/// alpha = dt_slots / period_slots  (capped at 1.0 → full reset)
/// new_ema = old + alpha × (new_price − old)
///
/// On first reading (old == 0), returns new_price directly.
/// Uses integer arithmetic scaled by period_slots to avoid floats.
pub fn ema_update(old: i64, new_price: i64, dt_slots: u64, period_slots: u64) -> i64 {
    if old == 0 || period_slots == 0 {
        return new_price;
    }
    if dt_slots >= period_slots {
        return new_price; // enough time elapsed → full reset
    }
    let diff = new_price.saturating_sub(old);
    // adjustment = diff * dt_slots / period_slots  (integer, truncated)
    let adjustment = (diff as i128)
        .checked_mul(dt_slots as i128)
        .unwrap_or(0)
        .checked_div(period_slots as i128)
        .unwrap_or(0);
    old.saturating_add(adjustment as i64)
}

/// Calculate gradual base shift
/// Shift is proportional to confirmed growth
/// Capped at MAX_BASE_SHIFT_BPS per cycle
pub fn calculate_base_shift(
    current_base:     i64,
    confirmed_growth: i64,
) -> i64 {
    if current_base == 0 {
        return 0;
    }

    // Growth in bps
    let growth_bps = (confirmed_growth as i128)
        .checked_mul(BPS_DENOMINATOR as i128)
        .unwrap_or(0)
        / current_base as i128;

    // Cap shift at MAX_BASE_SHIFT_BPS
    let shift_bps = growth_bps
        .abs()
        .min(MAX_BASE_SHIFT_BPS as i128);

    // Calculate actual shift amount
    let shift = (current_base as i128)
        .checked_mul(shift_bps)
        .unwrap_or(0)
        / BPS_DENOMINATOR as i128;

    if confirmed_growth > 0 {
        shift as i64
    } else {
        -(shift as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fee_neutral() {
        // Exactly at base → fee = fee_max (top of V-shape)
        let fee = calculate_fee(100, 100, 800, 400, 30, 100);
        assert_eq!(fee, 100); // fee_max at base
    }

    #[test]
    fn test_fee_approaching_up() {
        // SOL +4% (50% of 8% threshold) → fee should be below max
        let base  = 86_000_000i64;
        let price = 89_440_000i64; // +4%
        let fee = calculate_fee(price, base, 800, 400, 30, 100);
        assert!(fee < 100); // Below max
        assert!(fee >= 30); // At or above min
    }

    #[test]
    fn test_fee_exceeded_up() {
        // SOL +10% (exceeded 8% threshold) → fee at min
        let base  = 86_000_000i64;
        let price = 94_600_000i64; // +10%
        let fee = calculate_fee(price, base, 800, 400, 30, 100);
        assert_eq!(fee, 30); // At min
    }

    #[test]
    fn test_fee_approaching_down() {
        // SOL -3% (75% of 4% threshold) → fee should also slide toward min (V-shape)
        let base  = 86_000_000i64;
        let price = 83_420_000i64; // -3%
        let fee = calculate_fee(price, base, 800, 400, 30, 100);
        assert!(fee < 100); // Below max (V-shape: deviation → lower fee)
        assert!(fee >= 30); // At or above min
    }

    #[test]
    fn test_fee_exceeded_down() {
        // SOL -5% (exceeded 4% threshold) → fee at min (symmetric V-shape)
        let base  = 86_000_000i64;
        let price = 81_700_000i64; // -5%
        let fee = calculate_fee(price, base, 800, 400, 30, 100);
        assert_eq!(fee, 30); // At min — arbitrageurs incentivised to restore price
    }

    #[test]
    fn test_fee_symmetry() {
        // Equal % deviation in opposite directions should produce same fee
        // (assuming equal thresholds)
        let base  = 100_000_000i64;
        let up    = 104_000_000i64; // +4%
        let down  = 96_000_000i64;  // -4%
        let fee_up   = calculate_fee(up,   base, 400, 400, 30, 100);
        let fee_down = calculate_fee(down, base, 400, 400, 30, 100);
        assert_eq!(fee_up, fee_down); // Same deviation → same fee
    }

    #[test]
    fn test_threshold_state_neutral() {
        let state = calculate_threshold_state(100, 100, 800, 400);
        assert_eq!(state, ThresholdState::Neutral);
    }

    #[test]
    fn test_threshold_state_approaching_up() {
        // +4% of 8% threshold = 50%
        let base  = 86_000_000i64;
        let price = 89_440_000i64;
        let state = calculate_threshold_state(price, base, 800, 400);
        match state {
            ThresholdState::ApproachingUp(pct) => {
                assert!(pct >= 45 && pct <= 55);
            }
            _ => panic!("Expected ApproachingUp"),
        }
    }

    #[test]
    fn test_threshold_exceeded() {
        let base  = 86_000_000i64;
        let price = 94_600_000i64; // +10% > 8% threshold
        let state = calculate_threshold_state(price, base, 800, 400);
        assert_eq!(state, ThresholdState::ExceededUp);
    }

    #[test]
    fn test_twap_layer_all_up() {
        assert!(check_twap_layer(95, 90, 88, 100));
    }

    #[test]
    fn test_twap_layer_not_aligned() {
        // Medium higher than short = not aligned
        assert!(!check_twap_layer(95, 98, 88, 100));
    }

    #[test]
    fn test_confidence_narrow() {
        // Price $86, confidence $0.50 = 0.58% < 2% threshold
        assert!(check_confidence_layer(86_000_000, 500_000));
    }

    #[test]
    fn test_confidence_wide() {
        // Price $86, confidence $3.00 = 3.48% > 2% threshold
        assert!(!check_confidence_layer(86_000_000, 3_000_000));
    }
}
