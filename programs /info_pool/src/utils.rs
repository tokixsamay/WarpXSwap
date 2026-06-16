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
        return fee_max;
    };

    if threshold_bps == 0 {
        return fee_max;
    }

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
pub fn should_block_inflow(state: &ThresholdState) -> bool {
    matches!(state, ThresholdState::ExceededUp | ThresholdState::ExceededDown)
}

/// Check TWAP layer confirmation.
/// All three timeframes must trend same direction AND the current price must
/// deviate meaningfully from the long-term TWAP (Bug #8 fix).
///
/// Bug #8 fix: added minimum deviation check.
/// Previously, even a tiny 0.01% wobble that happened to be monotonic across
/// all three timeframes would confirm the TWAP layer — causing false threshold
/// triggers on near-flat markets.
/// Now requires current−twap_long spread to be >= TWAP_MIN_DEVIATION_BPS (0.10%).
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

    if !all_up && !all_down {
        return false;
    }

    // Bug #8: require minimum relative deviation between current price and the
    // long-term TWAP before confirming. This filters out flat-market noise where
    // prices happen to form a monotonic sequence but are all within 0.1% of each
    // other, which is statistically meaningless for threshold detection.
    if twap_long > 0 {
        let spread = current.saturating_sub(twap_long).unsigned_abs() as u128;
        let spread_bps = spread
            .checked_mul(BPS_DENOMINATOR as u128)
            .unwrap_or(0)
            .checked_div(twap_long as u128)
            .unwrap_or(0);
        if spread_bps < TWAP_MIN_DEVIATION_BPS as u128 {
            return false;
        }
    }

    true
}

/// Check volume layer confirmation across VOLUME_MIN_PERIODS consecutive periods.
///
/// Bug #9 fix: uses the 3-element volume_history for multi-period trend checking.
/// The old implementation only compared volume_24h vs volume_prev (1 period),
/// which could be satisfied by a single high-volume outlier candle followed by
/// any subsequent value — giving false confirmations.
///
/// New behaviour: all VOLUME_MIN_PERIODS (3) consecutive transitions must show
/// at least 10% growth.  Requires:
///   history[1] >= history[0] × 1.10
///   history[2] >= history[1] × 1.10
///   volume_current >= history[2] × 1.10
///
/// Any zero in the history means insufficient data → returns false.
///
/// `volume_history[0]` = 3 periods ago (oldest)
/// `volume_history[1]` = 2 periods ago
/// `volume_history[2]` = 1 period ago
/// `volume_current`    = current period
pub fn check_volume_layer(
    volume_current: u64,
    volume_history: &[u64; 3],
) -> bool {
    // Require all history slots to be populated
    if volume_history.iter().any(|&v| v == 0) {
        return false;
    }
    if volume_current == 0 {
        return false;
    }

    // Each transition must be ≥10% growth
    let check_period = |prev: u64, curr: u64| -> bool {
        let min_increase = prev.saturating_add(prev / 10);
        curr >= min_increase
    };

    check_period(volume_history[0], volume_history[1])
        && check_period(volume_history[1], volume_history[2])
        && check_period(volume_history[2], volume_current)
}

/// Check confidence interval layer.
pub fn check_confidence_layer(
    price:      i64,
    confidence: u64,
) -> bool {
    if price <= 0 {
        return false;
    }
    let max_confidence = (price.unsigned_abs())
        .checked_mul(CONFIDENCE_RATIO_BPS)
        .unwrap_or(u64::MAX)
        / BPS_DENOMINATOR;

    confidence <= max_confidence
}

/// Time-weighted Exponential Moving Average update.
pub fn ema_update(old: i64, new_price: i64, dt_slots: u64, period_slots: u64) -> i64 {
    if old == 0 || period_slots == 0 {
        return new_price;
    }
    if dt_slots >= period_slots {
        return new_price;
    }
    let diff = new_price.saturating_sub(old);
    let adjustment = (diff as i128)
        .checked_mul(dt_slots as i128)
        .unwrap_or(0)
        .checked_div(period_slots as i128)
        .unwrap_or(0);
    old.saturating_add(adjustment as i64)
}

/// Calculate gradual base shift.
pub fn calculate_base_shift(
    current_base:     i64,
    confirmed_growth: i64,
) -> i64 {
    if current_base == 0 {
        return 0;
    }

    let growth_bps = (confirmed_growth as i128)
        .checked_mul(BPS_DENOMINATOR as i128)
        .unwrap_or(0)
        / current_base as i128;

    let shift_bps = growth_bps
        .abs()
        .min(MAX_BASE_SHIFT_BPS as i128);

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
        let fee = calculate_fee(100, 100, 800, 400, 30, 100);
        assert_eq!(fee, 100);
    }

    #[test]
    fn test_fee_approaching_up() {
        let base  = 86_000_000i64;
        let price = 89_440_000i64;
        let fee = calculate_fee(price, base, 800, 400, 30, 100);
        assert!(fee < 100);
        assert!(fee >= 30);
    }

    #[test]
    fn test_fee_exceeded_up() {
        let base  = 86_000_000i64;
        let price = 94_600_000i64;
        let fee = calculate_fee(price, base, 800, 400, 30, 100);
        assert_eq!(fee, 30);
    }

    #[test]
    fn test_fee_approaching_down() {
        let base  = 86_000_000i64;
        let price = 83_420_000i64;
        let fee = calculate_fee(price, base, 800, 400, 30, 100);
        assert!(fee < 100);
        assert!(fee >= 30);
    }

    #[test]
    fn test_fee_exceeded_down() {
        let base  = 86_000_000i64;
        let price = 81_700_000i64;
        let fee = calculate_fee(price, base, 800, 400, 30, 100);
        assert_eq!(fee, 30);
    }

    #[test]
    fn test_fee_symmetry() {
        let base  = 100_000_000i64;
        let up    = 104_000_000i64;
        let down  = 96_000_000i64;
        let fee_up   = calculate_fee(up,   base, 400, 400, 30, 100);
        let fee_down = calculate_fee(down, base, 400, 400, 30, 100);
        assert_eq!(fee_up, fee_down);
    }

    #[test]
    fn test_threshold_state_neutral() {
        let state = calculate_threshold_state(100, 100, 800, 400);
        assert_eq!(state, ThresholdState::Neutral);
    }

    #[test]
    fn test_threshold_state_approaching_up() {
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
        let price = 94_600_000i64;
        let state = calculate_threshold_state(price, base, 800, 400);
        assert_eq!(state, ThresholdState::ExceededUp);
    }

    #[test]
    fn test_twap_layer_all_up_with_deviation() {
        // +5% deviation from long TWAP — should confirm
        let long    = 100_000_000i64;
        let medium  = 102_000_000i64;
        let short   = 104_000_000i64;
        let current = 105_000_000i64;
        assert!(check_twap_layer(short, medium, long, current));
    }

    #[test]
    fn test_twap_layer_noise_rejected() {
        // All aligned but deviation < 0.10% — should be rejected (Bug #8)
        let long    = 100_000_000i64;
        let medium  = 100_000_050i64;
        let short   = 100_000_080i64;
        let current = 100_000_090i64;
        assert!(!check_twap_layer(short, medium, long, current));
    }

    #[test]
    fn test_twap_layer_not_aligned() {
        assert!(!check_twap_layer(95, 98, 88, 100));
    }

    #[test]
    fn test_confidence_narrow() {
        assert!(check_confidence_layer(86_000_000, 500_000));
    }

    #[test]
    fn test_confidence_wide() {
        assert!(!check_confidence_layer(86_000_000, 3_000_000));
    }

    #[test]
    fn test_volume_layer_3_period_rising() {
        // 3 consecutive ≥10% increases + current (Bug #9)
        let history = [100u64, 115, 130];
        assert!(check_volume_layer(145, &history));
    }

    #[test]
    fn test_volume_layer_single_spike_rejected() {
        // Single high value surrounded by flat — old code would pass, new fails
        let history = [100u64, 200, 100]; // history[0]→[1] rises, [1]→[2] drops
        assert!(!check_volume_layer(115, &history));
    }

    #[test]
    fn test_volume_layer_zero_history_rejected() {
        let history = [0u64, 0, 0];
        assert!(!check_volume_layer(100, &history));
    }
  }
  
