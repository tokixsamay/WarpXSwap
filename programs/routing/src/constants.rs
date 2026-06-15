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
/// Pool must have at least this much of asset_out
pub const MIN_LIQUIDITY: u64 = 1_000_000;

/// Threshold approaching — 50%+ triggers P2
pub const P2_THRESHOLD_PCT: u8 = 50;

/// Info Pool Program ID
pub const INFO_POOL_PROGRAM_ID: &str =
    "9MXoZpzQZzvURN1S1EARJLaDhFuGw3RAppQMYvGTcmPo";

/// Pool Program ID
pub const POOL_PROGRAM_ID: &str =
    "4AXtXF5VWeWKLqP6vHKPpjoc7wQ8r4duDqZ4CENtzsqZ";
