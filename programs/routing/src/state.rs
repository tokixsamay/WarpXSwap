use anchor_lang::prelude::*;

// ── ROUTER CONFIG ─────────────────────────────────
#[account]
pub struct RouterConfig {
    /// Info Pool Program ID
    pub info_pool_program: Pubkey,
    /// Pool Program ID
    pub pool_program:      Pubkey,
    /// Router active
    pub is_active:         bool,
    /// PDA bump
    pub bump:              u8,
}

impl RouterConfig {
    pub const LEN: usize = 8 + 32 + 32 + 1 + 1;
}

// ── PRIORITY LEVEL ────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Priority {
    /// Threshold exceeded — highest preference
    P1Exceeded,
    /// Threshold 50%+ approached
    P2Approaching,
    /// Neutral — no special preference
    P3Neutral,
}

// ── POOL CANDIDATE ────────────────────────────────
// Internal struct for routing calculation
#[derive(Clone, Debug)]
pub struct PoolCandidate {
    pub pool:            Pubkey,
    pub asset_out_fee:   u16,
    pub pool_weight:     u64,
    pub priority:        Priority,
    pub liquidity:       u64,
    pub is_blocked:      bool,
    /// All 3 confirmation layers active for asset_out
    pub all_confirmed:   bool,
    /// Volume layer confirmed for asset_out
    pub volume_confirmed: bool,
    /// Oracle price of asset_in from InfoPool (not user-supplied)
    pub rate_in:         u64,
    /// Oracle price of asset_out from InfoPool (not user-supplied)
    pub rate_out:        u64,
}

// ── FIND BEST POOL PARAMS ─────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct FindBestPoolParams {
    /// Asset user is sending (e.g. ETH)
    pub asset_in:         Pubkey,
    /// Asset user wants (e.g. BTC)
    pub asset_out:        Pubkey,
    /// Amount user is sending
    pub amount_in:        u64,
    /// Max fee user will accept (basis points)
    pub max_fee_bps:      u16,
    /// Pool candidates to check (from client)
    pub candidate_pools:  Vec<Pubkey>,
    // rate_in / rate_out removed — router reads oracle prices from InfoPool directly
}

// ── QUOTE PARAMS ──────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct QuoteParams {
    pub asset_in:   Pubkey,
    pub asset_out:  Pubkey,
    pub amount_in:  u64,
    pub pool:       Pubkey,
    // rate_in / rate_out removed — router reads oracle prices from InfoPool directly
}

// ── EXECUTE ROUTE PARAMS ─────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct ExecuteRouteParams {
    pub asset_in:        Pubkey,
    pub asset_out:       Pubkey,
    pub amount_in:       u64,
    pub min_amount_out:  u64,
    pub max_fee_bps:     u16,
    pub candidate_pools: Vec<Pubkey>,
    // rate_in / rate_out removed — router reads oracle prices from InfoPool directly
    // and passes them to the Pool CPI, eliminating user-supplied rate manipulation
}

// ── ROUTE RESULT ──────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct RouteResult {
    /// Best pool selected
    pub best_pool:       Pubkey,
    /// Expected output amount
    pub expected_out:    u64,
    /// Fee in basis points
    pub fee_bps:         u16,
    /// Priority level assigned
    pub priority:        u8,
    /// Pool weight of selected pool
    pub pool_weight:     u64,
    /// Volume layer confirmed (volume_24h ≥ volume_prev × 1.1) for asset_out
    pub volume_confirmed: bool,
    /// All 3 layers confirmed (TWAP + Volume + Confidence) for asset_out.
    /// When true the base-price shift engine is active; fees are at their
    /// lowest and price discovery is considered high-confidence.
    pub all_confirmed:   bool,
}

// ── QUOTE RESULT ─────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct QuoteResult {
    pub pool:            Pubkey,
    pub amount_out:      u64,
    pub fee_amount:      u64,
    pub fee_bps:         u16,
    /// Volume layer confirmed for the quoted asset_out
    pub volume_confirmed: bool,
    /// All 3 confirmation layers active for the quoted asset_out
    pub all_confirmed:   bool,
}

