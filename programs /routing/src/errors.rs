use anchor_lang::prelude::*;

#[error_code]
pub enum RoutingError {
    #[msg("No eligible pool found for this swap")]
    NoPoolFound,

    #[msg("No direct route found for this asset pair")]
    NoDirectRoute,

    #[msg("Asset inflow is blocked in all eligible pools")]
    AllPoolsBlocked,

    #[msg("Fee exceeds user maximum tolerance")]
    FeeExceedsMax,

    #[msg("Insufficient liquidity in best pool")]
    InsufficientLiquidity,

    #[msg("Slippage too high — price impact exceeds limit")]
    SlippageTooHigh,

    #[msg("Router is not active")]
    RouterNotActive,

    #[msg("No candidate pools provided")]
    NoCandidates,

    #[msg("Asset not allowed in any candidate pool")]
    AssetNotAllowedAnywhere,

    #[msg("Asset interaction not allowed")]
    InteractionNotAllowed,

    #[msg("Math overflow in routing calculation")]
    MathOverflow,

    #[msg("Invalid amount — must be greater than zero")]
    InvalidAmount,

    #[msg("Too many candidate pools (max 20)")]
    TooManyCandidates,

    /// Bug #6 fix: all remaining candidate pools have Pyth oracle data older than
    /// ORACLE_STALENESS_SLOTS.  The crank must run and push fresh prices to
    /// InfoPool before routing can resume.
    #[msg("All candidate pools have stale oracle data — InfoPool crank must update prices")]
    StaleOracleData,
}
