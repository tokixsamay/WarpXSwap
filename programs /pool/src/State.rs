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
    /// Pool weight — informational only (routing tiebreaker in InfoPool).
    /// Incremented by out_fee_amount (native units) on every swap — not used
    /// for fee-claim gating (that is now handled per-asset by AssetAccount.fee_balance).
    /// Do NOT use for arithmetic that crosses asset boundaries: values are
    /// in mixed native units (lamports, micro-USDC, etc.) and are not comparable.
    pub pool_weight:  u64,
    /// Pool active or paused
    pub is_active:    bool,
    /// Bump for PDA
    pub bump:         u8,

    // ── POOL-WIDE FEE DISTRIBUTION ────────────────
    // Only outgoing swap fees (from ALL assets) flow into a single pool-level
    // accumulator.  Every LP in the pool — regardless of which asset they
    // deposited — earns proportionally from every swap.
    //
    // Bug #2 fix: all values are USD-normalised so that SOL lamports and
    // USDC micro-tokens are comparable.  Fee amounts are converted using the
    // oracle price at swap time; deposit amounts are converted at deposit time.
    //
    // Model:
    //   pool_fps                   — monotonically increasing accumulator,
    //                                scaled by FEE_SCALE (1e9).
    //                                Bumped on every swap:
    //                                  Δfps = fee_usd × FEE_SCALE
    //                                         / pool_total_lp_deposited_usd
    //   pool_total_lp_deposited_usd — USD-normalised sum of all active LP deposits:
    //                                  Σ (amount × oracle_price
    //                                     / (ORACLE_PRICE_SCALE × 10^decimals))
    //                                  Incremented on deposit, decremented on exit.
    //   pool_total_lp_deposited    — native-unit sum (kept for reference; NOT used
    //                                as the fps denominator after Bug #2 fix).
    //
    // LP claim formula:
    //   claimable_usd = lp_deposit.amount_usd × (pool_fps − fee_debt) / FEE_SCALE
    //   claimable_native = claimable_usd × ORACLE_PRICE_SCALE × 10^decimals
    //                      / asset.oracle_price
    //
    // Fee-claim gating: asset.fee_balance >= claimable_native (Bug #1 fix).
    // Payout: LP is paid from their deposited asset's vault.
    pub pool_fps:                    u64,
    /// Sum of all active LP principal deposits across ALL assets — native units only.
    /// Kept for reference; the fps denominator is pool_total_lp_deposited_usd.
    pub pool_total_lp_deposited:     u64,
    /// USD-normalised sum of all active LP deposits (Bug #2 fix).
    /// This is the authoritative fps denominator; avoids decimal-mismatch across
    /// assets with different token precisions (e.g. SOL 9-decimal vs USDC 6-decimal).
    pub pool_total_lp_deposited_usd: u64,
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
    // pool_fps: 8
    // pool_total_lp_deposited: 8
    // pool_total_lp_deposited_usd: 8  ← Bug #2 addition
    pub const LEN: usize = 8 + 2 + 32 + 32 + 8 + 8 + 1 + 1 + 8 + 8 + 8;
}

// ── ASSET ACCOUNT ─────────────────────────────────
#[account]
pub struct AssetAccount {
    /// Pool this asset belongs to
    pub pool:             Pubkey,
    /// Token mint address
    pub mint:             Pubkey,
    /// Current amount in pool vault (principal + swap inflows + unclaimed fees)
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
    /// Current base price for threshold calculation.
    /// Set by Info Pool — shifts with genuine growth.
    /// Bug #4 fix: changed from u64 to i64 to match InfoPool's AssetInfo.current_base
    /// and calculate_fee / calculate_threshold_state signatures (which use i64).
    pub current_base:     i64,
    /// Assets this asset allows to interact with
    /// Max 10 allowed assets
    pub allowed:          Vec<Pubkey>,
    /// Inflow blocked (threshold exceeded + at max%)
    pub is_blocked:       bool,
    /// Threshold state for routing priority
    pub threshold_state:  ThresholdState,
    /// Latest oracle price pushed by InfoPool (USD per whole token × ORACLE_PRICE_SCALE).
    /// e.g. SOL at $150 → oracle_price = 150_000_000.
    /// Must be > 0 before any swap can execute.
    pub oracle_price:     u64,
    /// Slot number when oracle_price was last updated via push_oracle_price CPI (Bug #3).
    /// Swap handler rejects with OraclePriceStale if
    ///   current_slot − oracle_price_slot > MAX_ORACLE_STALENESS_SLOTS.
    pub oracle_price_slot: u64,
    /// True if this asset is a stablecoin (USDC, USDT, PYUSD).
    pub is_stable:        bool,
    /// LP-chosen flat fee (basis points) used when is_stable = true.
    pub static_fee_bps:   u16,
    /// Token decimal precision (e.g. 9 for SOL, 6 for USDC/USDT/PYUSD).
    /// Required to USD-normalise deposits for the pool_fps denominator (Bug #2 fix).
    pub decimals:         u8,
    /// Bump for PDA
    pub bump:             u8,
    /// DEPRECATED — no longer updated by swap.rs.
    /// Kept for struct-size compatibility only.  All fee distribution now flows
    /// through PoolAccount.pool_fps (pool-wide, USD-normalised).
    pub _deprecated_fps:  u64,
    /// Total deposited principal for this asset (native units).
    /// Incremented on LP deposit only — NOT on swap inflows.
    pub total_deposited:  u64,
    /// Accumulated outgoing swap fees collected in this asset's vault (Bug #1 fix).
    /// Incremented by out_fee_amount on every swap where this is the outgoing asset.
    /// Decremented when an LP claims or exits fee-share from this vault.
    /// Guards claim_fees / public_exit: claimable_native <= fee_balance prevents
    /// WeightError underflows caused by mixing different asset units in pool_weight.
    pub fee_balance:      u64,
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
    //  8 current_base  (i64, same byte width as u64)
    // 324 allowed  (4 len prefix + 32×10)
    //  1 is_blocked
    //  2 threshold_state
    //  8 oracle_price
    //  8 oracle_price_slot  ← Bug #3 addition
    //  1 is_stable
    //  2 static_fee_bps
    //  1 decimals          ← Bug #2 addition
    //  1 bump
    //  8 _deprecated_fps
    //  8 total_deposited
    //  8 fee_balance       ← Bug #1 addition
    pub const LEN: usize = 8 + 32 + 32 + 8 + 1 + 1 + 2 + 2 + 2 + 2 + 2 + 8 + 324 + 1 + 2 + 8 + 8 + 1 + 2 + 1 + 1 + 8 + 8 + 8;
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
// Fee-share fields (pool-wide USD-normalised model):
//   fee_debt    — snapshot of pool.pool_fps at deposit/settle time.
//   amount_usd  — USD-normalised principal (Bug #2 fix). Used as numerator in
//                 claimable_usd = amount_usd × (pool_fps − fee_debt) / FEE_SCALE.
//   pending_fees — USD-valued fees settled at re-deposit time so they are not
//                  lost when fee_debt is reset.
#[account]
pub struct LpDepositAccount {
    /// Pool this deposit belongs to
    pub pool:         Pubkey,
    /// Asset mint (one PDA per asset per user per pool)
    pub asset:        Pubkey,
    /// The depositor wallet
    pub depositor:    Pubkey,
    /// Cumulative net deposit in native token units (deposits − public-exits).
    /// Principal only — does NOT include unclaimed fees.
    pub amount:       u64,
    /// USD-normalised principal (Bug #2 fix).
    /// amount_usd = amount × oracle_price / (ORACLE_PRICE_SCALE × 10^decimals).
    /// Used as the per-LP numerator in the pool_fps fee formula.
    pub amount_usd:   u64,
    /// PDA bump
    pub bump:         u8,
    /// Snapshot of pool.pool_fps at the time of last deposit or settle.
    pub fee_debt:     u64,
    /// USD-valued fees settled at re-deposit so no earnings are lost.
    /// Stored in USD units (matches pool_fps scale); converted to native at payout.
    pub pending_fees: u64,
}

impl LpDepositAccount {
    // 8  discriminator
    // 32 pool
    // 32 asset
    // 32 depositor
    //  8 amount
    //  8 amount_usd  ← Bug #2 addition
    //  1 bump
    //  8 fee_debt
    //  8 pending_fees
    pub const LEN: usize = 8 + 32 + 32 + 32 + 8 + 8 + 1 + 8 + 8;
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
    /// Bug #4 fix: changed from u64 to i64 to match AssetAccount.current_base (i64)
    /// and InfoPool's AssetInfo.current_base (i64).
    pub initial_base:   i64,
    pub allowed:        Vec<Pubkey>,
    /// True if this asset is a stablecoin (USDC, USDT, PYUSD).
    pub is_stable:      bool,
    /// Static fee in basis points used when is_stable = true.
    pub static_fee_bps: u16,
    /// Token decimal precision (e.g. 9 for SOL, 6 for USDC/USDT/PYUSD).
    /// Required for USD normalisation of pool_fps (Bug #2 fix).
    pub decimals:       u8,
}
