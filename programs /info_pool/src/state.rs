use anchor_lang::prelude::*;

// ── THRESHOLD STATE ────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, Debug)]
pub enum ThresholdState {
    Neutral,
    ApproachingUp(u8),
    ApproachingDown(u8),
    ExceededUp,
    ExceededDown,
}

// ── LAYER CONFIRMATION ─────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct LayerConfirmation {
    pub twap_confirmed:       bool,
    pub volume_confirmed:     bool,
    pub confidence_confirmed: bool,
    pub all_confirmed:        bool,
    pub last_confirmed:       i64,
}

// ── PYTH FEED DATA ────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct PythFeedData {
    pub mint:         Pubkey,
    /// 30-minute TWAP (scaled by 10^6)
    pub twap_short:   i64,
    /// 4-hour TWAP
    pub twap_medium:  i64,
    /// 24-hour TWAP
    pub twap_long:    i64,
    /// Current 24h volume (USD scaled)
    pub volume_24h:   u64,
    /// Bug #9 fix: 3-period rolling volume history for multi-period trend confirmation.
    /// volume_history[0] = 3 periods ago (oldest)
    /// volume_history[1] = 2 periods ago
    /// volume_history[2] = 1 period ago (most recent completed period)
    /// Rotated in push_volume: each call shifts history forward and appends the
    /// previous volume_24h.  check_volume_layer requires all 3 pairwise transitions
    /// to show ≥10% growth before confirming the volume layer.
    /// Replacing the old single volume_prev field, which only verified 1 period
    /// and could be fooled by a single high-volume candle.
    pub volume_history: [u64; 3],
    /// Confidence interval (Pyth native)
    pub confidence:   u64,
    /// Current spot price (Pyth price × 10^6)
    pub price:        i64,
    /// Last updated slot
    pub last_updated: i64,
}

// ── ASSET INFO (per asset in pool) ────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct AssetInfo {
    pub mint:            Pubkey,
    pub current_pct:     u16,
    pub current_base:    i64,
    pub threshold_up:    u16,
    pub threshold_down:  u16,
    pub fee_min:         u16,
    pub fee_max:         u16,
    pub current_fee:     u16,
    pub max_pct_min:     u8,
    pub max_pct_max:     u8,
    pub allowed:         Vec<Pubkey>,
    pub is_blocked:      bool,
    pub threshold_state: ThresholdState,
    pub layer_status:    LayerConfirmation,
    pub pyth_data:       PythFeedData,
    /// Pyth V2 feed ID (32-byte).
    pub pyth_feed_id:    [u8; 32],
    pub is_stable:       bool,
    pub static_fee_bps:  u16,
}

impl AssetInfo {
    pub const MAX_ALLOWED: usize = 10;
    // Space per asset (approximate):
    //   32 + 2 + 8 + 2+2+2+2+2 + 1+1 + (4+320) + 1 + 2 + 13 (LayerConfirmation)
    //   + PythFeedData (32+8*3+8+24+8+8+8 = 120) + 32 + 1 + 2 = ~551 bytes
    // 10 assets × 551 = 5510 → InfoPoolAccount::LEN bumped to 5800
}

// ── INFO POOL ACCOUNT ─────────────────────────────
#[account]
pub struct InfoPoolAccount {
    pub pool_id:      Pubkey,
    pub authority:    Pubkey,
    pub assets:       Vec<AssetInfo>,
    pub pool_size:    u64,
    pub pool_weight:  u64,
    pub last_updated: i64,
    pub bump:         u8,
}

impl InfoPoolAccount {
    // Bumped from 5500 → 5800 to accommodate PythFeedData.volume_history [u64; 3]
    // (+24 bytes per asset × 10 assets = +240 bytes; rounded up for safety).
    pub const LEN: usize = 5800;
    pub const MAX_ASSETS: usize = 10;
}

// ── RESPONSE TYPES ────────────────────────────────
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
        
