use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::PoolError;

// ═══════════════════════════════════════════════════
// GOVERNANCE UPDATE FEE RANGE
// Called by Governance Program via CPI after vote passes
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey, new_min: u16, new_max: u16)]
pub struct GovernanceUpdateFeeRange<'info> {
    #[account(
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), mint.as_ref()],
        bump = asset.bump,
    )]
    pub asset: Account<'info, AssetAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_update_fee_range(
    ctx: Context<GovernanceUpdateFeeRange>,
    mint: Pubkey,
    new_min: u16,
    new_max: u16,
) -> Result<()> {
    // Verify caller is the Governance program PDA (owned by governance program)
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(PoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id,
        PoolError::NotGovernance
    );

    require!(new_min < new_max, PoolError::FeeOutOfRange);
    require!(new_min >= MIN_FEE_BPS, PoolError::FeeOutOfRange);
    require!(new_max <= MAX_FEE_BPS, PoolError::FeeOutOfRange);

    let asset = &mut ctx.accounts.asset;

    let old_min = asset.fee_min;
    let old_max = asset.fee_max;

    asset.fee_min = new_min;
    asset.fee_max = new_max;

    // Clamp current fee within new range
    if asset.current_fee < new_min {
        asset.current_fee = new_min;
    } else if asset.current_fee > new_max {
        asset.current_fee = new_max;
    }

    emit!(FeeRangeUpdated {
        pool:    ctx.accounts.pool.key(),
        mint,
        old_min,
        old_max,
        new_min,
        new_max,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// GOVERNANCE UPDATE THRESHOLD
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey, new_up: u16, new_down: u16)]
pub struct GovernanceUpdateThreshold<'info> {
    #[account(
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), mint.as_ref()],
        bump = asset.bump,
    )]
    pub asset: Account<'info, AssetAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_update_threshold(
    ctx: Context<GovernanceUpdateThreshold>,
    mint: Pubkey,
    new_up: u16,
    new_down: u16,
) -> Result<()> {
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(PoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id,
        PoolError::NotGovernance
    );

    require!(
        new_up > 0 && new_down > 0,
        PoolError::InvalidThreshold
    );

    let asset = &mut ctx.accounts.asset;

    let old_up   = asset.threshold_up;
    let old_down = asset.threshold_down;

    asset.threshold_up   = new_up;
    asset.threshold_down = new_down;

    // Reset threshold state after change
    asset.threshold_state = ThresholdState::Neutral;
    asset.is_blocked      = false;

    emit!(ThresholdUpdated {
        pool:     ctx.accounts.pool.key(),
        mint,
        old_up,
        old_down,
        new_up,
        new_down,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// GOVERNANCE UPDATE MAX PCT
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey, new_min: u8, new_max: u8)]
pub struct GovernanceUpdateMaxPct<'info> {
    #[account(
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), mint.as_ref()],
        bump = asset.bump,
    )]
    pub asset: Account<'info, AssetAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_update_max_pct(
    ctx: Context<GovernanceUpdateMaxPct>,
    mint: Pubkey,
    new_min: u8,
    new_max: u8,
) -> Result<()> {
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(PoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id,
        PoolError::NotGovernance
    );

    require!(new_min < new_max, PoolError::InvalidMaxPct);
    require!(new_max <= 100, PoolError::InvalidMaxPct);

    let asset = &mut ctx.accounts.asset;

    asset.max_pct_min = new_min;
    asset.max_pct_max = new_max;

    emit!(MaxPctUpdated {
        pool:    ctx.accounts.pool.key(),
        mint,
        new_min,
        new_max,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// GOVERNANCE ADD ASSET
// Called by Governance after an AddAsset proposal passes.
// Executor pays rent for the new AssetAccount PDA.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(params: AddAssetParams)]
pub struct GovernanceAddAsset<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        init,
        payer = payer,
        space = AssetAccount::LEN,
        seeds = [ASSET_SEED, pool.key().as_ref(), params.mint.as_ref()],
        bump
    )]
    pub asset: Account<'info, AssetAccount>,

    pub governance_authority: Signer<'info>,

    #[account(mut)]
    pub payer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler_governance_add_asset(
    ctx: Context<GovernanceAddAsset>,
    params: AddAssetParams,
) -> Result<()> {
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(PoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id,
        PoolError::NotGovernance
    );

    require!(
        params.max_pct_min < params.max_pct_max,
        PoolError::InvalidMaxPct
    );
    // Stablecoins use a single static fee and have no threshold blocking.
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

    let asset = &mut ctx.accounts.asset;

    asset.pool              = ctx.accounts.pool.key();
    asset.mint              = params.mint;
    asset.amount            = 0;
    asset.oracle_price      = 0;
    // Bug #3 fix: initialise oracle_price_slot = 0 (sentinel "never pushed").
    // Swap handler requires oracle_price_slot > 0 before the staleness window
    // check, so swaps are blocked until the first InfoPool price push.
    asset.oracle_price_slot = 0;
    // Bug #10 fix: renamed from fees_per_share — deprecated; retained for compat.
    asset._deprecated_fps   = 0;
    // Bug #1 fix: per-asset fee vault tracker — incremented by swap, decremented on claim.
    asset.fee_balance       = 0;
    // Bug #2 fix: decimal precision for USD normalisation in fps accumulator.
    asset.decimals          = params.decimals;
    asset.total_deposited   = 0;
    asset.max_pct_min     = params.max_pct_min;
    asset.max_pct_max     = params.max_pct_max;
    asset.fee_min         = params.fee_min;
    asset.fee_max         = params.fee_max;
    // For stablecoins: use LP-set static fee directly (no V-shape curve).
    // For volatile assets: start at midpoint of fee_min..fee_max range.
    // Use u32 arithmetic to avoid u16 overflow during addition before halving.
    asset.current_fee     = if params.is_stable {
        params.static_fee_bps
    } else {
        ((params.fee_min as u32 + params.fee_max as u32) / 2) as u16
    };
    asset.threshold_up    = params.threshold_up;
    asset.threshold_down  = params.threshold_down;
    asset.current_base    = params.initial_base;
    asset.allowed         = params.allowed;
    asset.is_blocked      = false;
    asset.threshold_state = ThresholdState::Neutral;
    asset.is_stable       = params.is_stable;
    asset.static_fee_bps  = params.static_fee_bps;
    asset.bump            = ctx.bumps.asset;

    emit!(GovernanceAssetAdded {
        pool:    ctx.accounts.pool.key(),
        mint:    params.mint,
        fee_min: params.fee_min,
        fee_max: params.fee_max,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// GOVERNANCE REMOVE ASSET
// Called by Governance after a RemoveAsset proposal passes.
// Rent lamports are returned to the executor (rent_recipient).
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct GovernanceRemoveAsset<'info> {
    #[account(
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        mut,
        close = rent_recipient,
        seeds = [ASSET_SEED, pool.key().as_ref(), asset.mint.as_ref()],
        bump = asset.bump,
        // Cannot remove if vault still holds tokens
        constraint = asset.amount == 0 @ PoolError::InsufficientBalance,
        // Cannot remove if LPs still have tracked deposits (would strand their LpDepositAccount PDAs)
        constraint = asset.total_deposited == 0 @ PoolError::InsufficientBalance,
        // Cannot remove base asset
        constraint = asset.mint != pool.base_asset @ PoolError::CannotRemoveBaseAsset,
    )]
    pub asset: Account<'info, AssetAccount>,

    pub governance_authority: Signer<'info>,

    #[account(mut)]
    pub rent_recipient: SystemAccount<'info>,
}

pub fn handler_governance_remove_asset(
    ctx: Context<GovernanceRemoveAsset>,
) -> Result<()> {
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(PoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id,
        PoolError::NotGovernance
    );

    let mint = ctx.accounts.asset.mint;

    emit!(GovernanceAssetRemoved {
        pool: ctx.accounts.pool.key(),
        mint,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// GOVERNANCE SET ALLOWANCE
// Called by Governance after an UpdateAllowance proposal passes.
// Updates which target assets a given asset permits interaction with.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(asset_mint: Pubkey, target_mint: Pubkey, allowed: bool)]
pub struct GovernanceSetAllowance<'info> {
    #[account(
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), asset_mint.as_ref()],
        bump = asset.bump,
    )]
    pub asset: Account<'info, AssetAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_governance_set_allowance(
    ctx: Context<GovernanceSetAllowance>,
    _asset_mint: Pubkey,
    target_mint: Pubkey,
    allowed: bool,
) -> Result<()> {
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(PoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id,
        PoolError::NotGovernance
    );

    let asset = &mut ctx.accounts.asset;

    if allowed {
        if !asset.allowed.contains(&target_mint) {
            require!(
                asset.allowed.len() < AssetAccount::MAX_ALLOWED,
                PoolError::TooManyAllowed
            );
            asset.allowed.push(target_mint);
        }
    } else {
        asset.allowed.retain(|&x| x != target_mint);
    }

    emit!(GovernanceAllowanceUpdated {
        pool:        ctx.accounts.pool.key(),
        asset:       asset.mint,
        target_mint,
        allowed,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// GOVERNANCE SET INFLOW BLOCKED
// Called by Governance after a SetInflowBlocked proposal passes.
// Allows governance to manually block or unblock inflow for an asset
// as an emergency circuit-breaker (e.g. making Pool 3 public by unblocking).
// Unblocking resets threshold_state to Neutral so the crank resumes normally.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey, blocked: bool)]
pub struct GovernanceSetInflowBlocked<'info> {
    #[account(
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), mint.as_ref()],
        bump = asset.bump,
    )]
    pub asset: Account<'info, AssetAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_governance_set_inflow_blocked(
    ctx: Context<GovernanceSetInflowBlocked>,
    mint: Pubkey,
    blocked: bool,
) -> Result<()> {
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(PoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id,
        PoolError::NotGovernance
    );

    let asset       = &mut ctx.accounts.asset;
    let old_blocked = asset.is_blocked;
    asset.is_blocked = blocked;

    // Unblocking resets threshold state so the crank re-evaluates cleanly.
    if !blocked {
        asset.threshold_state = ThresholdState::Neutral;
    }

    emit!(GovernanceInflowBlockedSet {
        pool:        ctx.accounts.pool.key(),
        mint,
        old_blocked,
        blocked,
    });

    Ok(())
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct FeeRangeUpdated {
    pub pool:    Pubkey,
    pub mint:    Pubkey,
    pub old_min: u16,
    pub old_max: u16,
    pub new_min: u16,
    pub new_max: u16,
}

#[event]
pub struct ThresholdUpdated {
    pub pool:     Pubkey,
    pub mint:     Pubkey,
    pub old_up:   u16,
    pub old_down: u16,
    pub new_up:   u16,
    pub new_down: u16,
}

#[event]
pub struct MaxPctUpdated {
    pub pool:    Pubkey,
    pub mint:    Pubkey,
    pub new_min: u8,
    pub new_max: u8,
}

#[event]
pub struct GovernanceAssetAdded {
    pub pool:    Pubkey,
    pub mint:    Pubkey,
    pub fee_min: u16,
    pub fee_max: u16,
}

#[event]
pub struct GovernanceAssetRemoved {
    pub pool: Pubkey,
    pub mint: Pubkey,
}

#[event]
pub struct GovernanceAllowanceUpdated {
    pub pool:        Pubkey,
    pub asset:       Pubkey,
    pub target_mint: Pubkey,
    pub allowed:     bool,
}

#[event]
pub struct GovernanceInflowBlockedSet {
    pub pool:        Pubkey,
    pub mint:        Pubkey,
    pub old_blocked: bool,
    pub blocked:     bool,
          }
