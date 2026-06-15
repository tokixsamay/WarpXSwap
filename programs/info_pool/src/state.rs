use anchor_lang::prelude::*;

// ── THRESHOLD STATE ────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, Debug)]
pub enum ThresholdState {
    Neutral,
    ApproachingUp(u8),    // 0-100 = % of threshold reached
    ApproachingDown(u8),
    ExceededUp,
    ExceededDown,
}

// ── LAYER CONFIRMATION ─────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct LayerConfirmation {
    /// Layer 1: TWAP all timeframes aligned
    pub twap_confirmed:   bool,
    /// Layer 2: Volume consistently rising
    pub volume_confirmed: bool,
    /// Layer 3: Confidence interval narrow
    pub confidence_confirmed: bool,
    /// All 3 layers confirmed = genuine growth
    pub all_confirmed:    bool,
    /// Last confirmation timestamp
    pub last_confirmed:   i64,
}

// ── PYTH FEED DATA ────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct PythFeedData {
    pub mint:          Pubkey,
    /// 30-minute TWAP (scaled by 10^6)
    pub twap_short:    i64,
    /// 4-hour TWAP
    pub twap_medium:   i64,
    /// 24-hour TWAP
    pub twap_long:     i64,
    /// 24h volume (USD scaled)
    pub volume_24h:    u64,
    /// Previous 24h volume (for trend check)
    pub volume_prev:   u64,
    /// Confidence interval (Pyth native)
    pub confidence:    u64,
    /// Current spot price
    pub price:         i64,
    /// Last updated slot
    pub last_updated:  i64,
}

// ── ASSET INFO (per asset in pool) ────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct AssetInfo {
    pub mint:             Pubkey,
    /// Current % of pool (basis points: 10000 = 100%)
    pub current_pct:      u16,
    /// Dynamic threshold base — shifts with genuine growth
    pub current_base:     i64,
    /// Upper threshold (basis points)
    pub threshold_up:     u16,
    /// Lower threshold (basis points)
    pub threshold_down:   u16,
    /// Fee range min (basis points)
    pub fee_min:          u16,
    /// Fee range max (basis points)
    pub fee_max:          u16,
    /// Current calculated fee
    pub current_fee:      u16,
    /// Max % range min
    pub max_pct_min:      u8,
    /// Max % range max
    pub max_pct_max:      u8,
    /// Allowed interaction mints
    pub allowed:          Vec<Pubkey>,
    /// Inflow blocked
    pub is_blocked:       bool,
    /// Current threshold state
    pub threshold_state:  ThresholdState,
    /// 3-layer confirmation status
    pub layer_status:     LayerConfirmation,
    /// Pyth feed data
    pub pyth_data:        PythFeedData,
    /// Pyth V2 feed ID (32-byte hex decoded).
    /// Must be set via governance_set_pyth_feed_id before update_pyth_feeds is called.
    /// Zeros = not configured; update_pyth_feeds will reject with PythFeedNotConfigured.
    pub pyth_feed_id:     [u8; 32],
    /// Stablecoin flag — when true, fee is fixed at static_fee_bps set by LP.
    /// The V-shape dynamic fee curve is skipped entirely for stable assets.
    /// Pyth tracking and de-peg inflow blocking still apply.
    pub is_stable:        bool,
    /// Static fee for stablecoin assets (basis points).
    /// Only used when is_stable = true.  Set via governance_set_stable.
    pub static_fee_bps:   u16,
}

impl AssetInfo {
    pub const MAX_ALLOWED: usize = 10;
    // Space: 32+2+8+2+2+2+2+2+1+1+(4+320)+1+2+13+96+32+1+2 = ~527 bytes per asset
    // 10 assets × 527 = 5270; header ~69 → total ~5339 < LEN 5500 ✓
}

// ── INFO POOL ACCOUNT ─────────────────────────────
#[account]
pub struct InfoPoolAccount {
    /// Associated pool
    pub pool_id:      Pubkey,
    /// LP who initialized this InfoPool — used to authorize pre-governance setup calls
    pub authority:    Pubkey,
    /// All assets tracked
    pub assets:       Vec<AssetInfo>,
    /// Total pool value (base asset terms)
    pub pool_size:    u64,
    /// Pool weight score
    pub pool_weight:  u64,
    /// Last full update
    pub last_updated: i64,
    /// PDA bump
    pub bump:         u8,
}

impl InfoPoolAccount {
    // Precise layout:
    //   8 discriminant + 32 pool_id + 32 authority
    //   + 4 (vec prefix) + 524 * 10 assets
    //   + 8 pool_size + 8 pool_weight + 8 last_updated + 1 bump
    //   = 5301 → padded to 5500 for safety
    pub const LEN: usize = 5500;
    pub const MAX_ASSETS: usize = 10;
}

// ── RESPONSE TYPES (for CPI returns) ─────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct PoolStateResponse {
    pub pool_id:     Pubkey,
    pub pool_size:   u64,
    pub pool_weight: u64,
    pub assets:      Vec<AssetSummary>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct AssetSummary {
    pub mint:            Pubkey,
    pub current_pct:     u16,
    pub current_fee:     u16,
    pub is_blocked:      bool,
    pub threshold_state: ThresholdState,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct ThresholdStateResponse {
    pub mint:            Pubkey,
    pub state:           ThresholdState,
    pub current_fee:     u16,
    pub is_blocked:      bool,
    pub layer_status:    LayerConfirmation,
}
