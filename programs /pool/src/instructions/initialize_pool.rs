use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::PoolError;

#[derive(Accounts)]
pub struct InitializePool<'info> {
    #[account(
        init,
        payer = authority,
        space = PoolAccount::LEN,
        seeds = [POOL_SEED, authority.key().as_ref()],
        bump
    )]
    pub pool: Account<'info, PoolAccount>,

    /// Base asset mint (e.g. SOL)
    pub base_asset_mint: Account<'info, anchor_spl::token::Mint>,

    /// Pool authority (founding member for public, owner for private)
    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<InitializePool>,
    pool_type: PoolType,
) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    let authority = &ctx.accounts.authority;

    pool.pool_type               = pool_type;
    pool.owner                   = authority.key();
    pool.base_asset              = ctx.accounts.base_asset_mint.key();
    pool.total_value             = 0;
    // pool_weight tracks cumulative outgoing swap fees retained in vaults.
    // It starts at 0 — no fees have been collected yet.
    // It is incremented ONLY by out_fee_amount in swap.rs (STEP 9),
    // and decremented when LPs claim or exit their fee share.
    // Principal deposits/withdrawals must NOT touch pool_weight.
    pool.pool_weight             = 0;
    pool.pool_fps                    = 0;
    pool.pool_total_lp_deposited     = 0;
    // Bug #2 fix: USD-normalised LP deposit denominator for fps accumulator.
    pool.pool_total_lp_deposited_usd = 0;
    pool.is_active                   = true;
    pool.bump                    = ctx.bumps.pool;

    emit!(PoolInitialized {
        pool:       pool.key(),
        pool_type:  pool.pool_type.clone(),
        owner:      pool.owner,
        base_asset: pool.base_asset,
    });

    Ok(())
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct PoolInitialized {
    pub pool:       Pubkey,
    pub pool_type:  PoolType,
    pub owner:      Pubkey,
    pub base_asset: Pubkey,
      }
