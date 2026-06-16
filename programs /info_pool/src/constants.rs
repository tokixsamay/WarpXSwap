/// Seeds
pub const INFO_POOL_SEED: &[u8] = b"info_pool";

/// Pyth price max staleness (slots)
/// ~400ms per slot, 10 slots = ~4 seconds
pub const PYTH_MAX_STALENESS: i64 = 10;

/// Pyth confidence threshold
/// If confidence > price × CONFIDENCE_RATIO_BPS / BPS_DENOMINATOR → wide interval
/// 200 = 2% (if confidence > 2% of price = suspicious)
pub const CONFIDENCE_RATIO_BPS: u64 = 200;

/// TWAP timeframes (in slots)
/// Short:  30 min = 30 * 60 / 0.4 = 4,500 slots
/// Medium: 4 hr   = 4 * 60 * 60 / 0.4 = 36,000 slots
/// Long:   24 hr  = 24 * 60 * 60 / 0.4 = 216,000 slots
pub const TWAP_SHORT_SLOTS:  u64 = 4_500;
pub const TWAP_MEDIUM_SLOTS: u64 = 36_000;
pub const TWAP_LONG_SLOTS:   u64 = 216_000;

/// Bug #8 fix: minimum relative deviation (basis points) required for TWAP
/// layer confirmation. When current price is within TWAP_MIN_DEVIATION_BPS
/// of the long-term TWAP, the TWAP layer is treated as noise (not confirmed).
/// 10 bps = 0.10% — prices must deviate at least 0.1% from the long TWAP
/// before a threshold move is treated as genuine.
pub const TWAP_MIN_DEVIATION_BPS: u64 = 10;

/// Volume consistency — minimum periods needed (used in check_volume_layer).
/// With volume_history [u64; 3] + current, we verify VOLUME_MIN_PERIODS=3
/// consecutive ≥10% increases before confirming the volume layer.
pub const VOLUME_MIN_PERIODS: usize = 3;

/// Threshold base shift — max per update
/// 100 bps = 1% max shift per confirmation cycle
pub const MAX_BASE_SHIFT_BPS: u64 = 100;

/// Fee sensitivity (0-100)
pub const FEE_SENSITIVITY: u64 = 80;

/// Basis points denominator
pub const BPS_DENOMINATOR: u64 = 10_000;

/// Pool program ID
pub const POOL_PROGRAM_ID: &str =
    "4AXtXF5VWeWKLqP6vHKPpjoc7wQ8r4duDqZ4CENtzsqZ";

/// Governance program ID
pub const GOVERNANCE_PROGRAM_ID: &str =
    "C1iFRYB3fw7Rq2i2JFruYLbJoGTxRb6ohYqerYBpUsLm";

/// Max % buffer check (same as pool program)
pub const MAX_PCT_BUFFER: u8 = 10;

pub const ROUTING_PROGRAM_ID: &str =
    "3fdt9Skkj52bMvutU56CuBMZhrUsaStXBxGNtDPVCRSG";

/// Oracle staleness threshold for InfoPool (in slots).
/// Used in routing to skip candidates whose Pyth data is stale (Bug #6 fix).
/// At ~400ms/slot, 150 slots ≈ 60 seconds.
pub const ORACLE_STALENESS_SLOTS: i64 = 150;
