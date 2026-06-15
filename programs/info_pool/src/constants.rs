/// Seeds
pub const INFO_POOL_SEED: &[u8] = b"info_pool";

/// Pyth price max staleness (slots)
/// ~400ms per slot, 10 slots = ~4 seconds
pub const PYTH_MAX_STALENESS: i64 = 10;

/// Pyth confidence threshold
/// If confidence > price * CONFIDENCE_RATIO → wide interval
/// 200 = 2% (if confidence > 2% of price = suspicious)
pub const CONFIDENCE_RATIO_BPS: u64 = 200;

/// TWAP timeframes (in slots)
/// Short:  30 min = 30 * 60 / 0.4 = 4,500 slots
/// Medium: 4 hr   = 4 * 60 * 60 / 0.4 = 36,000 slots
/// Long:   24 hr  = 24 * 60 * 60 / 0.4 = 216,000 slots
pub const TWAP_SHORT_SLOTS:  u64 = 4_500;
pub const TWAP_MEDIUM_SLOTS: u64 = 36_000;
pub const TWAP_LONG_SLOTS:   u64 = 216_000;

/// Volume consistency — minimum periods needed
pub const VOLUME_MIN_PERIODS: usize = 3;

/// Threshold base shift — max per update
/// 100 bps = 1% max shift per confirmation cycle
pub const MAX_BASE_SHIFT_BPS: u64 = 100;

/// Fee sensitivity (0-100)
/// How aggressively fee responds to threshold approach
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
