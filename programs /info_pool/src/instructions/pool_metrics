use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;

// ═══════════════════════════════════════════════════════════════════
// UPDATE POOL METRICS  (Bug #19 / Bug #21 fix)
//
// Pushes the Pool program's live `total_value` (as pool_size) and
// `pool_weight` into InfoPool so the Routing program's hard-rule
// filter can evaluate pool liveness:
//
//   pool_is_active = pool_liquidity > 0   (pool_liquidity = ip.pool_size)
//   filter_hard_rules → returns false when !pool_is_active
//
// Before this fix, info_pool.pool_size was initialised to 0 and
// NEVER written, so every pool was permanently excluded from routing.
// info_pool.pool_weight was hardcoded to 1_000_000 at init and never
// updated, so tie-breaking always produced stale results.
//
// Called by the crank once per pool after the 4-step per-asset cycle.
// ═══════════════════════════════════════════════════════════════════

#[derive(Accounts)]
pub struct UpdatePoolMetrics<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump  = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    /// Crank signer.  pool_size and pool_weight are public counters
    /// derived from the Pool program's on-chain state, so no extra
    /// authority restriction is needed beyond the crank signature.
    pub crank: Signer<'info>,
}

pub fn handler_update_pool_metrics(
    ctx:         Context<UpdatePoolMetrics>,
    pool_size:   u64,
    pool_weight: u64,
) -> Result<()> {
    let info_pool = &mut ctx.accounts.info_pool;

    info_pool.pool_size    = pool_size;
    info_pool.pool_weight  = pool_weight;
    info_pool.last_updated = Clock::get()?.unix_timestamp;

    emit!(PoolMetricsUpdated {
        pool_id:     info_pool.pool_id,
        pool_size,
        pool_weight,
    });

    Ok(())
}

// ── EVENTS ─────────────────────────────────────────────────────────
#[event]
pub struct PoolMetricsUpdated {
    pub pool_id:     Pubkey,
    pub pool_size:   u64,
    pub pool_weight: u64,
}
