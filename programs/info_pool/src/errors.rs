use anchor_lang::prelude::*;

#[error_code]
pub enum InfoPoolError {
    #[msg("Asset not found in Info Pool")]
    AssetNotFound,

    #[msg("Info Pool already initialized for this pool")]
    AlreadyInitialized,

    #[msg("Unauthorized caller — pool program only")]
    NotPoolProgram,

    #[msg("Unauthorized caller — governance only")]
    NotGovernance,

    #[msg("Unauthorized caller — crank only")]
    NotCrank,

    #[msg("Pyth price account mismatch")]
    PythAccountMismatch,

    #[msg("Pyth price is stale")]
    PythPriceStale,

    #[msg("Pyth confidence interval too wide")]
    PythConfidenceTooWide,

    #[msg("Invalid threshold values")]
    InvalidThreshold,

    #[msg("Math overflow in calculation")]
    MathOverflow,

    #[msg("Base price cannot be zero")]
    ZeroBasePrice,

    #[msg("Too many assets in Info Pool")]
    TooManyAssets,

    #[msg("Threshold base shift too large")]
    ShiftTooLarge,

    #[msg("Volume data insufficient for confirmation")]
    InsufficientVolumeData,

    #[msg("CPI to Pool program to update fee failed")]
    CpiUpdateFeeFailed,

    #[msg("CPI to Pool program to block inflow failed")]
    CpiBlockInflowFailed,

    #[msg("CPI to Pool program to unblock inflow failed")]
    CpiUnblockInflowFailed,

    #[msg("Pyth feed ID not configured — call governance_set_pyth_feed_id first")]
    PythFeedNotConfigured,

    #[msg("Static fee must be > 0 when marking asset as stable")]
    InvalidStaticFee,

    #[msg("Cannot set dynamic threshold on a stablecoin asset")]
    StableAssetThresholdChange,

    #[msg("Oracle price is zero or negative — Pyth data error")]
    InvalidOraclePrice,
}
