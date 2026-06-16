use anchor_lang::prelude::*;

pub mod instructions;
pub mod state;
pub mod errors;
pub mod constants;
pub mod utils;

use instructions::*;

declare_id!("9MXoZpzQZzvURN1S1EARJLaDhFuGw3RAppQMYvGTcmPo");

#[program]
pub mod info_pool_program {
    use super::*;

    // ── SETUP ─────────────────────────────────────

    pub fn initialize_info_pool(
        ctx: Context<InitializeInfoPool>,
        pool_id: Pubkey,
    ) -> Result<()> {
        instructions::initialize::handler(ctx, pool_id)
    }

    pub fn register_pool(
        ctx: Context<RegisterPool>,
    ) -> Result<()> {
        instructions::initialize::handler_register(ctx)
    }

    // ── PYTH FEED UPDATE ──────────────────────────
    // Called by off-chain crank every block (~400ms)

    pub fn update_pyth_feeds(
        ctx: Context<UpdatePythFeeds>,
        mint: Pubkey,
    ) -> Result<()> {
        instructions::pyth::handler_update_feeds(ctx, mint)
    }

    /// Push the latest Pyth spot price into Pool's AssetAccount.oracle_price.
    /// Call this AFTER update_pyth_feeds each tick so Pool's swap always uses
    /// the freshest oracle price without a circular CPI dep.
    ///
    /// Crank order: update_pyth_feeds → push_oracle_price_to_pool →
    ///              run_threshold_check → calculate_and_push_fee
    pub fn push_oracle_price_to_pool(
        ctx:  Context<PushOraclePriceToPool>,
        mint: Pubkey,
    ) -> Result<()> {
        instructions::pyth::handler_push_oracle_price(ctx, mint)
    }

    // ── CORE THRESHOLD LOGIC ──────────────────────

    pub fn run_threshold_check(
        ctx: Context<RunThresholdCheck>,
        mint: Pubkey,
    ) -> Result<()> {
        instructions::threshold::handler_check(ctx, mint)
    }

    pub fn update_threshold_base(
        ctx: Context<UpdateThresholdBase>,
        mint: Pubkey,
        confirmed_growth: i64,
    ) -> Result<()> {
        instructions::threshold::handler_update_base(ctx, mint, confirmed_growth)
    }

    // ── FEE CALCULATION ───────────────────────────

    pub fn calculate_and_push_fee(
        ctx: Context<CalculateAndPushFee>,
        mint: Pubkey,
    ) -> Result<()> {
        instructions::fee::handler(ctx, mint)
    }

    // ── READ INSTRUCTIONS (CPI only) ──────────────

    pub fn get_pool_state(
        ctx: Context<GetPoolState>,
    ) -> Result<PoolStateResponse> {
        instructions::read::handler_pool_state(ctx)
    }

    pub fn get_asset_fee(
        ctx: Context<GetAssetFee>,
        mint: Pubkey,
    ) -> Result<u16> {
        instructions::read::handler_asset_fee(ctx, mint)
    }

    pub fn get_threshold_state(
        ctx: Context<GetThresholdState>,
        mint: Pubkey,
    ) -> Result<ThresholdStateResponse> {
        instructions::read::handler_threshold_state(ctx, mint)
    }

    // ── GOVERNANCE UPDATES ────────────────────────

    pub fn governance_update_threshold(
        ctx: Context<GovernanceUpdateThreshold>,
        mint: Pubkey,
        new_up: u16,
        new_down: u16,
    ) -> Result<()> {
        instructions::governance::handler_update_threshold(ctx, mint, new_up, new_down)
    }

    pub fn governance_add_asset(
        ctx: Context<GovernanceAddAsset>,
        mint:           Pubkey,
        max_pct_min:    u8,
        max_pct_max:    u8,
        fee_min:        u16,
        fee_max:        u16,
        threshold_up:   u16,
        threshold_down: u16,
        initial_base:   i64,
        allowed:        Vec<Pubkey>,
        is_stable:      bool,
        static_fee_bps: u16,
    ) -> Result<()> {
        instructions::governance::handler_add_asset(
            ctx, mint, max_pct_min, max_pct_max,
            fee_min, fee_max, threshold_up, threshold_down,
            initial_base, allowed, is_stable, static_fee_bps,
        )
    }

    pub fn governance_remove_asset(
        ctx: Context<GovernanceRemoveAsset>,
        mint: Pubkey,
    ) -> Result<()> {
        instructions::governance::handler_remove_asset(ctx, mint)
    }

    pub fn governance_update_fee_range(
        ctx: Context<GovernanceUpdateFeeRange>,
        mint:    Pubkey,
        new_min: u16,
        new_max: u16,
    ) -> Result<()> {
        instructions::governance::handler_update_fee_range(ctx, mint, new_min, new_max)
    }

    pub fn governance_update_max_pct(
        ctx: Context<GovernanceUpdateMaxPct>,
        mint:    Pubkey,
        new_min: u8,
        new_max: u8,
    ) -> Result<()> {
        instructions::governance::handler_update_max_pct(ctx, mint, new_min, new_max)
    }

    pub fn governance_set_allowance(
        ctx:         Context<GovernanceSetAllowance>,
        asset_mint:  Pubkey,
        target_mint: Pubkey,
        allowed:     bool,
    ) -> Result<()> {
        instructions::governance::handler_set_allowance(ctx, asset_mint, target_mint, allowed)
    }

    /// Push the latest 24h trading volume (from DexScreener or similar)
    /// for a registered asset.  Called by the off-chain crank at a lower
    /// cadence than update_pyth_feeds (~60 s instead of ~400 ms).
    /// Rotates volume_prev ← volume_24h then writes the new value so the
    /// 3-layer volume check can compare consecutive windows.
    pub fn push_volume(
        ctx:        Context<PushVolume>,
        mint:       Pubkey,
        volume_24h: u64,
    ) -> Result<()> {
        instructions::pyth::handler_push_volume(ctx, mint, volume_24h)
    }

    /// Set (or rotate) the Pyth V2 feed ID for a registered asset.
    /// Must be called after governance_add_asset and before update_pyth_feeds.
    /// Accepted signers: governance program or InfoPool founding authority.
    pub fn governance_set_pyth_feed_id(
        ctx:          Context<GovernanceSetPythFeedId>,
        mint:         Pubkey,
        pyth_feed_id: [u8; 32],
    ) -> Result<()> {
        instructions::governance::handler_set_pyth_feed_id(ctx, mint, pyth_feed_id)
    }

    // ── POOL METRICS (Bug #19 / #21 fix) ──────────
    /// Push Pool program's live `total_value` (pool_size) and `pool_weight`
    /// into InfoPool so the Routing program's `pool_is_active` guard passes.
    /// Called by the crank once per pool after the 4-step per-asset cycle.
    pub fn update_pool_metrics(
        ctx:         Context<UpdatePoolMetrics>,
        pool_size:   u64,
        pool_weight: u64,
    ) -> Result<()> {
        instructions::pool_metrics::handler_update_pool_metrics(ctx, pool_size, pool_weight)
    }

    /// Mark an asset as a stablecoin and set its LP-chosen static fee.
    /// When is_stable = true: calculate_and_push_fee uses static_fee_bps directly;
    /// the V-shape dynamic fee curve is bypassed entirely.
    /// Pyth tracking and de-peg inflow-blocking continue to apply.
    /// Solana stablecoins: USDC, USDT, PYUSD.
    /// Accepted signers: governance program or InfoPool founding authority.
    pub fn governance_set_stable(
        ctx:            Context<GovernanceSetStable>,
        mint:           Pubkey,
        is_stable:      bool,
        static_fee_bps: u16,
    ) -> Result<()> {
        instructions::governance::handler_set_stable(ctx, mint, is_stable, static_fee_bps)
    }
                                              }
      
