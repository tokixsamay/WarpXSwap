use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::PoolError;

#[derive(Accounts)]
#[instruction(params: AddAssetParams)]
pub struct AddAsset<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        init,
        payer = authority,
        space = AssetAccount::LEN,
        seeds = [ASSET_SEED, pool.key().as_ref(), params.mint.as_ref()],
        bump
    )]
    pub asset: Account<'info, AssetAccount>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<AddAsset>,
    params: AddAssetParams,
) -> Result<()> {
    let pool = &ctx.accounts.pool;
    let asset = &mut ctx.accounts.asset;
    let authority = &ctx.accounts.authority;

    // ── AUTH CHECK ────────────────────────────────
    require!(
        authority.key() == pool.owner,
        PoolError::Unauthorized
    );

    // ── VALIDATION ────────────────────────────────
    require!(
        params.max_pct_min < params.max_pct_max,
        PoolError::InvalidMaxPct
    );
    // Stablecoins use a single static fee (fee_min == fee_max == static_fee_bps)
    // and have no threshold blocking logic (thresholds must be 0).
    // Volatile assets require fee_min < fee_max and non-zero thresholds.
    if params.is_stable {
        require!(
            params.threshold_up == 0 && params.threshold_down == 0,
            PoolError::InvalidThreshold
        );
    } else {
        require!(
            params.fee_min < params.fee_max,
            PoolError::FeeOutOfRange
        );
        require!(
            params.threshold_up > 0 && params.threshold_down > 0,
            PoolError::InvalidThreshold
        );
    }
    require!(
        params.allowed.len() <= AssetAccount::MAX_ALLOWED,
        PoolError::TooManyAllowed
    );

    // ── SET ASSET DATA ────────────────────────────
    asset.pool           = pool.key();
    asset.mint           = params.mint;
    asset.amount         = 0;
    asset.max_pct_min    = params.max_pct_min;
    asset.max_pct_max    = params.max_pct_max;
    asset.fee_min        = params.fee_min;
    asset.fee_max        = params.fee_max;
    asset.current_fee    = if params.is_stable {
        params.static_fee_bps
    } else {
        ((params.fee_min as u32 + params.fee_max as u32) / 2) as u16
    };
    asset.threshold_up   = params.threshold_up;
    asset.threshold_down = params.threshold_down;
    // Bug #4 fix: current_base is now i64 to match InfoPool's AssetInfo.current_base
    // and the calculate_fee / calculate_threshold_state signatures.
    asset.current_base   = params.initial_base;
    asset.allowed        = params.allowed;
    asset.is_blocked     = false;
    asset.threshold_state = ThresholdState::Neutral;
    // oracle_price starts at 0 — must be pushed by InfoPool before swap executes.
    asset.oracle_price    = 0;
    // oracle_price_slot starts at 0 — zero is the sentinel "never pushed" value.
    // Swap handler checks oracle_price_slot > 0 before the staleness window check
    // (Bug #3 fix), so this will correctly block swaps until the first price push.
    asset.oracle_price_slot = 0;
    asset.is_stable       = params.is_stable;
    asset.static_fee_bps  = params.static_fee_bps;
    // Bug #2 fix: token decimal precision for USD normalisation.
    asset.decimals        = params.decimals;
    asset.bump            = ctx.bumps.asset;
    // Bug #10 fix: renamed from fees_per_share — deprecated; kept for struct compat.
    asset._deprecated_fps = 0;
    asset.total_deposited = 0;
    // Bug #1 fix: per-asset fee vault balance tracker.
    // Incremented by out_fee_amount in swap.rs; decremented on claim/exit.
    asset.fee_balance     = 0;

    emit!(AssetAdded {
        pool:      pool.key(),
        mint:      params.mint,
        fee_min:   params.fee_min,
        fee_max:   params.fee_max,
        decimals:  params.decimals,
    });

    Ok(())
}

#[event]
pub struct AssetAdded {
    pub pool:     Pubkey,
    pub mint:     Pubkey,
    pub fee_min:  u16,
    pub fee_max:  u16,
    pub decimals: u8,
}
