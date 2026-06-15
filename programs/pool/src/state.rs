use anchor_lang::prelude::*;

// ── POOL TYPE ──────────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum PoolType {
    Public,
    Private,
}

// ── POOL ACCOUNT ──────────────────────────────────
#[account]
pub struct PoolAccount {
    /// Pool type: Public or Private
    pub pool_type:    PoolType,
    /// Owner (LP for private, founding member for public)
    pub owner:        Pubkey,
    /// Base asset mint (e.g. SOL)
    pub base_asset:   Pubkey,
    /// Total value in pool (in base asset terms, lamports)
    pub total_value:  u64,
    /// Pool weight — tracks value retention
    pub pool_weight:  u64,
    /// Pool active or paused
    pub is_active:    bool,
    /// Bump for PDA
    pub bump:         u8,
}

impl PoolAccount {
    // Base: 8 discriminator
    // pool_type: 1 + 1 = 2 (enum)
    // owner: 32
    // base_asset: 32
    // total_value: 8
    // pool_weight: 8
    // is_active: 1
    // bump: 1
    pub const LEN: usize = 8 + 2 + 32 + 32 + 8 + 8 + 1 + 1;
}

// ── ASSET ACCOUNT ─────────────────────────────────
#[account]
pub struct AssetAccount {
    /// Pool this asset belongs to
    pub pool:             Pubkey,
    /// Token mint address
    pub mint:             Pubkey,
    /// Current amount in pool
    pub amount:           u64,
    /// Max % range min (e.g. 20 = 20%)
    pub max_pct_min:      u8,
    /// Max % range max (e.g. 30 = 30%)
    pub max_pct_max:      u8,
    /// Fee range min (basis points, e.g. 50 = 0.5%)
    pub fee_min:          u16,
    /// Fee range max (basis points, e.g. 100 = 1%)
    pub fee_max:          u16,
    /// Current calculated fee (basis points)
    pub current_fee:      u16,
    /// Upper threshold (basis points, e.g. 800 = 8%)
    pub threshold_up:     u16,
    /// Lower threshold (basis points, e.g. 400 = 4%)
    pub threshold_down:   u16,
    /// Current base price for threshold calculation
    /// Set by Info Pool — shifts with genuine growth
    pub current_base:     u64,
    /// Assets this asset allows to interact with
    /// Max 10 allowed assets
    pub allowed:          Vec<Pubkey>,
    /// Inflow blocked (threshold exceeded + at max%)
    pub is_blocked:       bool,
    /// Threshold state for routing priority
    pub threshold_state:  ThresholdState,
    /// Latest oracle price pushed by InfoPool (same unit as Pyth price × 1e6).
    /// Must be > 0 before any swap can execute.
    /// Updated via update_oracle_price CPI from InfoPool after each Pyth tick.
    pub oracle_price:     u64,
    /// True if this asset is a stablecoin (USDC, USDT, PYUSD).
    /// Stablecoins use static_fee_bps instead of the V-shape dynamic fee.
    pub is_stable:        bool,
    /// LP-chosen flat fee (basis points) used when is_stable = true.
    pub static_fee_bps:   u16,
    /// Bump for PDA
    pub bump:             u8,
    /// Cumulative fee accumulator per unit of deposited principal.
    /// Scaled by FEE_SCALE (1e9).  Monotonically increasing — never decreases.
    /// Updated in swap.rs whenever a swap fee is earned on this asset.
    /// Single-asset model: only LPs who deposited THIS asset earn fees when
    /// this asset is swapped out of the pool.
    pub fees_per_share:   u64,
    /// Sum of all active LP principal deposits for this asset.
    /// Increased on deposit, decreased on public_exit (principal only, not fees).
    /// Used as the denominator when updating fees_per_share.
    pub total_deposited:  u64,
}

impl AssetAccount {
    // 8  discriminator
    // 32 pool
    // 32 mint
    //  8 amount
    //  1 max_pct_min
    //  1 max_pct_max
    //  2 fee_min
    //  2 fee_max
    //  2 current_fee
    //  2 threshold_up
    //  2 threshold_down
    //  8 current_base
    // 324 allowed  (4 len prefix + 32×10)
    //  1 is_blocked
    //  2 threshold_state
    //  8 oracle_price
    //  1 is_stable
    //  2 static_fee_bps
    //  1 bump
    //  8 fees_per_share
    //  8 total_deposited
    pub const LEN: usize = 8 + 32 + 32 + 8 + 1 + 1 + 2 + 2 + 2 + 2 + 2 + 8 + 324 + 1 + 2 + 8 + 1 + 2 + 1 + 8 + 8;
    pub const MAX_ALLOWED: usize = 10;
}

// ── THRESHOLD STATE ────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq)]
pub enum ThresholdState {
    /// Price within normal range
    Neutral,
    /// Price approaching upper threshold (0-100 = % of threshold)
    ApproachingUp(u8),
    /// Price approaching lower threshold
    ApproachingDown(u8),
    /// Upper threshold exceeded — inflow blocked
    ExceededUp,
    /// Lower threshold exceeded — inflow blocked
    ExceededDown,
}

// ── LP DEPOSIT ACCOUNT ────────────────────────────
// Tracks how much of a specific asset each LP has deposited into a pool.
// Seeds: [LP_DEPOSIT_SEED, pool, asset_mint, depositor]
//
// Created (init_if_needed) on first deposit; amount grows with each deposit
// and shrinks on public_exit.  PublicExit enforces amount >= withdrawal,
// preventing any LP from exiting more than they contributed.
//
// Fee-share fields (single-asset independent model):
//   fee_debt    — snapshot of asset.fees_per_share at the time of last
//                 deposit or fee-settle.  Claimable accrued fees =
//                 amount × (current_fps − fee_debt) / FEE_SCALE.
//   pending_fees — fees locked in at re-deposit time so they are not lost
//                 when fee_debt is reset to the current accumulator.
#[account]
pub struct LpDepositAccount {
    /// Pool this deposit belongs to
    pub pool:         Pubkey,
    /// Asset mint (one PDA per asset per user per pool)
    pub asset:        Pubkey,
    /// The depositor wallet
    pub depositor:    Pubkey,
    /// Cumulative net deposit (deposits − public-exits) for this asset.
    /// This is the principal only — does NOT include unclaimed fees.
    pub amount:       u64,
    /// PDA bump
    pub bump:         u8,
    /// Snapshot of asset.fees_per_share at the time of last deposit or settle.
    /// Accrued (but unpaid) fees = amount × (current_fps − fee_debt) / FEE_SCALE.
    pub fee_debt:     u64,
    /// Fees that have been settled into this record on a re-deposit.
    /// These are added to the formula result in public_exit so no earnings are lost.
    pub pending_fees: u64,
}

impl LpDepositAccount {
    // 8  discriminator
    // 32 pool
    // 32 asset
    // 32 depositor
    //  8 amount
    //  1 bump
    //  8 fee_debt
    //  8 pending_fees
    pub const LEN: usize = 8 + 32 + 32 + 32 + 8 + 1 + 8 + 8;
}

// ── ADD ASSET PARAMS ──────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct AddAssetParams {
    pub mint:           Pubkey,
    pub max_pct_min:    u8,
    pub max_pct_max:    u8,
    pub fee_min:        u16,
    pub fee_max:        u16,
    pub threshold_up:   u16,
    pub threshold_down: u16,
    pub initial_base:   u64,
    pub allowed:        Vec<Pubkey>,
    /// True if this asset is a stablecoin (USDC, USDT, PYUSD).
    pub is_stable:      bool,
    /// Static fee in basis points used when is_stable = true.
    pub static_fee_bps: u16,
}
