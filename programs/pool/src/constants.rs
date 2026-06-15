/// Max % buffer above hard cap before hard reject
/// e.g. hard cap 30%, buffer allows up to 40%
pub const MAX_PCT_BUFFER: u8 = 10;

/// Basis points denominator (10000 = 100%)
pub const BPS_DENOMINATOR: u64 = 10_000;

/// Minimum fee (basis points) — 0.01%
pub const MIN_FEE_BPS: u16 = 1;

/// Maximum fee (basis points) — 5%
pub const MAX_FEE_BPS: u16 = 500;

/// Threshold approaching — percentage of threshold
/// where proactive response begins
/// 0 = immediately, 100 = only at threshold
pub const THRESHOLD_APPROACH_START: u8 = 0;

/// Pool type discriminator seeds
pub const POOL_SEED:         &[u8] = b"pool";
pub const ASSET_SEED:        &[u8] = b"asset";
/// Per-user per-asset deposit tracking PDA seed
pub const LP_DEPOSIT_SEED:   &[u8] = b"lp_deposit";

/// Info Pool program ID
pub const INFO_POOL_PROGRAM_ID: &str =
    "9MXoZpzQZzvURN1S1EARJLaDhFuGw3RAppQMYvGTcmPo";

/// Governance program ID
pub const GOVERNANCE_PROGRAM_ID: &str =
    "C1iFRYB3fw7Rq2i2JFruYLbJoGTxRb6ohYqerYBpUsLm";

/// Sensitivity factor for fee calculation (0-100)
/// Higher = more aggressive fee response
pub const FEE_SENSITIVITY: u64 = 80;

/// Max assets per pool
pub const MAX_ASSETS: usize = 10;

/// Pool weight precision multiplier
pub const WEIGHT_PRECISION: u64 = 1_000_000;

/// Scale factor for the per-asset fee accumulator (fees_per_share).
/// fees_per_share is stored as (fee_tokens × FEE_SCALE / total_deposited).
/// 1e9 gives sub-lamport precision up to ~9.2e18 total fees before overflow.
pub const FEE_SCALE: u64 = 1_000_000_000;
