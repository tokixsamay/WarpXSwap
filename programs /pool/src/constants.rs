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
/// Pool vault PDA seed (Bug #5: vault accounts should be derived as PDAs with these seeds)
/// Seed pattern: [VAULT_SEED, pool.key().as_ref(), asset.mint.as_ref()]
/// NOTE: Existing vault creation instructions must be updated to use this pattern
/// so that the PDA constraint can be enforced on swap/deposit/withdraw contexts.
pub const VAULT_SEED:        &[u8] = b"vault";

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

/// Scale factor for the pool-wide fee accumulator (pool_fps).
/// pool_fps is stored as (fee_usd × FEE_SCALE / pool_total_lp_deposited_usd).
/// (Bug #10: formerly called fees_per_share; AssetAccount._deprecated_fps retained.)
/// 1e9 gives sub-cent precision up to ~9.2e18 total USD fees before overflow.
pub const FEE_SCALE: u64 = 1_000_000_000;

/// Minimum native-token amount for the FIRST deposit into a private pool.
/// Enforces a meaningful initial liquidity commitment and activates the
/// `InsufficientInitialValue` error path (Bug #23 fix).
/// Value: 1_000_000_000 base units (= 1 whole token at 9 decimals, e.g. 1 wSOL).
pub const MIN_PRIVATE_POOL_INITIAL_DEPOSIT: u64 = 1_000_000_000;

/// Oracle price scale — Pyth prices are stored as USD × 1_000_000.
/// e.g. SOL at $150 → oracle_price = 150_000_000.
/// Used to normalize token amounts to USD for pool_fps denominator (Bug #2 fix).
pub const ORACLE_PRICE_SCALE: u64 = 1_000_000;

/// Maximum oracle price age in slots before a swap is rejected (Bug #3 fix).
/// At ~400ms per slot, 150 slots ≈ 60 seconds.
/// If InfoPool's crank has not pushed a fresh price within this window,
/// the swap reverts with OraclePriceStale rather than executing at stale rates.
pub const MAX_ORACLE_STALENESS_SLOTS: u64 = 150;
