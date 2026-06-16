use anchor_lang::prelude::*;

#[error_code]
pub enum PoolError {
    #[msg("Asset not allowed in this pool")]
    AssetNotAllowed,

    #[msg("Interaction between these assets not allowed")]
    InteractionNotAllowed,

    #[msg("Asset inflow is blocked — threshold exceeded")]
    InflowBlocked,

    #[msg("Asset max % limit exceeded")]
    MaxPctExceeded,

    #[msg("Max % buffer exceeded")]
    MaxPctBufferExceeded,

    #[msg("Insufficient liquidity in pool")]
    InsufficientLiquidity,

    #[msg("Slippage tolerance exceeded")]
    SlippageExceeded,

    #[msg("Pool is not active")]
    PoolNotActive,

    #[msg("Unauthorized — not pool owner")]
    Unauthorized,

    #[msg("Unauthorized — not founding member")]
    NotFoundingMember,

    #[msg("Unauthorized — only Info Pool CPI allowed")]
    NotInfoPool,

    #[msg("Unauthorized — only Governance CPI allowed")]
    NotGovernance,

    #[msg("Fee out of range (min/max bounds)")]
    FeeOutOfRange,

    #[msg("Threshold value invalid")]
    InvalidThreshold,

    #[msg("Max % values invalid (min must be < max)")]
    InvalidMaxPct,

    #[msg("Too many allowed assets (max 10)")]
    TooManyAllowed,

    #[msg("Asset already exists in pool")]
    AssetAlreadyExists,

    #[msg("Asset not found in pool")]
    AssetNotFound,

    #[msg("Cannot remove base asset")]
    CannotRemoveBaseAsset,

    #[msg("Pool type does not support this operation")]
    PoolTypeMismatch,

    #[msg("Withdrawal amount exceeds balance")]
    InsufficientBalance,

    #[msg("Percentage must be between 1 and 100")]
    InvalidPercentage,

    #[msg("Math overflow")]
    MathOverflow,

    #[msg("Pool weight calculation error")]
    WeightError,

    #[msg("Private pool creation requires minimum initial liquidity of $100,000 USD")]
    InsufficientInitialValue,

    #[msg("Oracle rate must be greater than zero")]
    InvalidRate,

    #[msg("Oracle price not set — InfoPool must push price before swap")]
    OraclePriceNotSet,

    #[msg("Withdrawal amount exceeds your recorded deposit — cannot exit more than you put in")]
    ExceedsDeposit,

    /// Bug #3 fix: oracle price pushed by InfoPool is older than MAX_ORACLE_STALENESS_SLOTS.
    /// The crank must be running and InfoPool's push_oracle_price must have fired within
    /// the staleness window before any swap can execute.
    #[msg("Oracle price is stale — InfoPool crank has not updated within the staleness window")]
    OraclePriceStale,

    /// Bug #1 fix: fee claim or exit would drain more from the per-asset fee_balance
    /// than was ever collected from outgoing swaps on this asset.
    /// This replaces the old pool_weight WeightError (which mixed units across assets).
    #[msg("Insufficient fee balance — claimed amount exceeds collected fees for this asset")]
    InsufficientFeeBalance,
}
