/// Seeds
pub const ROUTER_SEED: &[u8] = b"router";

/// Max candidate pools per request
pub const MAX_CANDIDATES: usize = 20;

/// Basis points denominator
pub const BPS_DENOMINATOR: u64 = 10_000;

/// Max price impact allowed (basis points)
/// 300 = 3% max slippage
pub const MAX_PRICE_IMPACT_BPS: u16 = 300;

/// Min liquidity required (in lamports)
pub const MIN_LIQUIDITY: u64 = 1_000_000;

/// Threshold approaching — 50%+ triggers P2
pub const P2_THRESHOLD_PCT: u8 = 50;

/// Info Pool Program ID
pub const INFO_POOL_PROGRAM_ID: &str =
    "9MXoZpzQZzvURN1S1EARJLaDhFuGw3RAppQMYvGTcmPo";

/// Pool Program ID
pub const POOL_PROGRAM_ID: &str =
    "4AXtXF5VWeWKLqP6vHKPpjoc7wQ8r4duDqZ4CENtzsqZ";

/// Bug #6 fix: maximum age (in Solana slots) of InfoPool Pyth data before a
/// routing candidate is considered stale and skipped.
/// At ~400ms per slot, 150 slots ≈ 60 seconds.
/// A candidate whose last_updated slot is older than this is excluded from
/// routing results rather than routing through potentially stale prices.
pub const ORACLE_STALENESS_SLOTS: i64 = 150;
