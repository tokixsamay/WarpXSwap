use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::InfoPoolError;

// ═══════════════════════════════════════════════════
// GOVERNANCE UPDATE THRESHOLD
// Called by Governance Program after vote passes
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct GovernanceUpdateThreshold<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_update_threshold(
    ctx: Context<GovernanceUpdateThreshold>,
    mint: Pubkey,
    new_up: u16,
    new_down: u16,
) -> Result<()> {
    // Verify caller is the Governance program PDA (owned by governance program)
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id,
        InfoPoolError::NotGovernance
    );

    require!(
        new_up > 0 && new_down > 0,
        InfoPoolError::InvalidThreshold
    );

    let info_pool = &mut ctx.accounts.info_pool;

    let asset = info_pool.assets
        .iter_mut()
        .find(|a| a.mint == mint)
        .ok_or(InfoPoolError::AssetNotFound)?;

    require!(
        !asset.is_stable,
        InfoPoolError::StableAssetThresholdChange
    );

    let old_up   = asset.threshold_up;
    let old_down = asset.threshold_down;

    asset.threshold_up   = new_up;
    asset.threshold_down = new_down;

    // Reset state after threshold change
    asset.threshold_state = ThresholdState::Neutral;
    asset.is_blocked      = false;
    asset.layer_status    = LayerConfirmation::default();

    emit!(ThresholdUpdatedByGovernance {
        pool_id:  info_pool.pool_id,
        mint,
        old_up,
        old_down,
        new_up,
        new_down,
    });

    Ok(())
}

#[event]
pub struct ThresholdUpdatedByGovernance {
    pub pool_id:  Pubkey,
    pub mint:     Pubkey,
    pub old_up:   u16,
    pub old_down: u16,
    pub new_up:   u16,
    pub new_down: u16,
}

// ═══════════════════════════════════════════════════
// GOVERNANCE ADD ASSET
// Registers a new asset in the Info Pool's inline Vec so the
// 3-layer Pyth confirmation engine begins tracking it immediately.
// Must be called atomically alongside Pool::governance_add_asset.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct GovernanceAddAsset<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_add_asset(
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
    // Accept governance program PDA (owner check) OR the InfoPool's founding
    // authority (LP) so the setup script can register assets before governance
    // proposals are in place.  Governance-only enforcement is V2 work.
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id
            || ctx.accounts.governance_authority.key() == ctx.accounts.info_pool.authority,
        InfoPoolError::NotGovernance
    );
    // Volatile assets require non-zero thresholds for the 3-layer engine.
    // Stablecoins skip threshold logic entirely (thresholds must be 0).
    if is_stable {
        require!(
            threshold_up == 0 && threshold_down == 0,
            InfoPoolError::InvalidThreshold
        );
    } else {
        require!(
            threshold_up > 0 && threshold_down > 0,
            InfoPoolError::InvalidThreshold
        );
    }
    require!(initial_base > 0, InfoPoolError::ZeroBasePrice);

    let info_pool = &mut ctx.accounts.info_pool;

    require!(
        info_pool.assets.len() < InfoPoolAccount::MAX_ASSETS,
        InfoPoolError::TooManyAssets
    );
    require!(
        !info_pool.assets.iter().any(|a| a.mint == mint),
        InfoPoolError::AlreadyInitialized
    );

    info_pool.assets.push(AssetInfo {
        mint,
        current_pct:     0,
        current_base:    initial_base,
        threshold_up,
        threshold_down,
        fee_min,
        fee_max,
        current_fee:     (fee_min + fee_max) / 2,
        max_pct_min,
        max_pct_max,
        allowed,
        is_blocked:      false,
        threshold_state: ThresholdState::Neutral,
        layer_status:    LayerConfirmation::default(),
        pyth_data:       PythFeedData {
            mint,
            ..PythFeedData::default()
        },
        // Zeros — caller must invoke governance_set_pyth_feed_id next.
        pyth_feed_id:    [0u8; 32],
        // Propagated from the governance proposal payload.
        // For stablecoins, threshold_up/down are both 0 (no blocking logic).
        is_stable,
        static_fee_bps,
    });

    emit!(AssetRegisteredByGovernance {
        pool_id: info_pool.pool_id,
        mint,
        threshold_up,
        threshold_down,
        fee_min,
        fee_max,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// GOVERNANCE REMOVE ASSET
// Removes an asset from the Info Pool's inline Vec.
// Must be called atomically alongside Pool::governance_remove_asset.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct GovernanceRemoveAsset<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_remove_asset(
    ctx: Context<GovernanceRemoveAsset>,
    mint: Pubkey,
) -> Result<()> {
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id,
        InfoPoolError::NotGovernance
    );

    let info_pool = &mut ctx.accounts.info_pool;

    let before = info_pool.assets.len();
    info_pool.assets.retain(|a| a.mint != mint);

    require!(
        info_pool.assets.len() < before,
        InfoPoolError::AssetNotFound
    );

    emit!(AssetRemovedByGovernance {
        pool_id: info_pool.pool_id,
        mint,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// GOVERNANCE UPDATE FEE RANGE
// Updates fee_min / fee_max on the AssetInfo so the next
// calculate_and_push_fee crank call uses the new bounds.
// Must be called atomically alongside Pool::governance_update_fee_range.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct GovernanceUpdateFeeRange<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_update_fee_range(
    ctx: Context<GovernanceUpdateFeeRange>,
    mint:    Pubkey,
    new_min: u16,
    new_max: u16,
) -> Result<()> {
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id,
        InfoPoolError::NotGovernance
    );
    require!(new_min < new_max, InfoPoolError::InvalidThreshold);

    let info_pool = &mut ctx.accounts.info_pool;

    let asset = info_pool.assets
        .iter_mut()
        .find(|a| a.mint == mint)
        .ok_or(InfoPoolError::AssetNotFound)?;

    let old_min = asset.fee_min;
    let old_max = asset.fee_max;

    asset.fee_min = new_min;
    asset.fee_max = new_max;

    // Clamp the live fee so the next crank push is within the new range
    asset.current_fee = asset.current_fee.max(new_min).min(new_max);

    emit!(FeeRangeUpdatedByGovernance {
        pool_id: info_pool.pool_id,
        mint,
        old_min,
        old_max,
        new_min,
        new_max,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// GOVERNANCE SET PYTH FEED ID
// Sets the per-asset Pyth V2 feed ID (32-byte hex decoded).
// Must be called for every asset before the crank can invoke
// update_pyth_feeds; until set the instruction reverts with
// PythFeedNotConfigured.
//
// Accepted signers: governance program PDA (owner check) OR InfoPool
// founding authority so the setup script can configure feeds without
// a full governance vote.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct GovernanceSetPythFeedId<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_set_pyth_feed_id(
    ctx: Context<GovernanceSetPythFeedId>,
    mint:         Pubkey,
    pyth_feed_id: [u8; 32],
) -> Result<()> {
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id
            || ctx.accounts.governance_authority.key() == ctx.accounts.info_pool.authority,
        InfoPoolError::NotGovernance
    );

    let info_pool = &mut ctx.accounts.info_pool;

    let asset = info_pool.assets
        .iter_mut()
        .find(|a| a.mint == mint)
        .ok_or(InfoPoolError::AssetNotFound)?;

    asset.pyth_feed_id = pyth_feed_id;

    emit!(PythFeedIdSet {
        pool_id: info_pool.pool_id,
        mint,
    });

    Ok(())
}

#[event]
pub struct PythFeedIdSet {
    pub pool_id: Pubkey,
    pub mint:    Pubkey,
}

// ═══════════════════════════════════════════════════
// GOVERNANCE SET STABLE
// Marks an asset as a stablecoin and sets its LP-chosen static fee.
// When is_stable = true:
//   • calculate_and_push_fee uses static_fee_bps directly (no V-shape curve).
//   • Pyth tracking and de-peg inflow-blocking still apply.
// When is_stable = false (unset):
//   • Reverts to V-shape dynamic fee; static_fee_bps is ignored.
//
// Accepted signers: governance program PDA (owner check) OR InfoPool
// founding authority (LP-auth bootstrap).
// Solana stablecoins: USDC, USDT, PYUSD.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct GovernanceSetStable<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_set_stable(
    ctx:            Context<GovernanceSetStable>,
    mint:           Pubkey,
    is_stable:      bool,
    static_fee_bps: u16,
) -> Result<()> {
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id
            || ctx.accounts.governance_authority.key() == ctx.accounts.info_pool.authority,
        InfoPoolError::NotGovernance
    );

    // When marking as stable, static_fee_bps must be non-zero
    if is_stable {
        require!(static_fee_bps > 0, InfoPoolError::InvalidStaticFee);
    }

    let info_pool = &mut ctx.accounts.info_pool;

    let asset = info_pool.assets
        .iter_mut()
        .find(|a| a.mint == mint)
        .ok_or(InfoPoolError::AssetNotFound)?;

    let old_is_stable      = asset.is_stable;
    let old_static_fee_bps = asset.static_fee_bps;

    asset.is_stable      = is_stable;
    asset.static_fee_bps = static_fee_bps;

    // Apply static fee immediately so the next crank push sees the right value
    if is_stable {
        asset.current_fee = static_fee_bps;
    }

    emit!(StableConfigUpdated {
        pool_id:            info_pool.pool_id,
        mint,
        old_is_stable,
        new_is_stable:      is_stable,
        old_static_fee_bps,
        new_static_fee_bps: static_fee_bps,
    });

    Ok(())
}

#[event]
pub struct StableConfigUpdated {
    pub pool_id:            Pubkey,
    pub mint:               Pubkey,
    pub old_is_stable:      bool,
    pub new_is_stable:      bool,
    pub old_static_fee_bps: u16,
    pub new_static_fee_bps: u16,
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct AssetRegisteredByGovernance {
    pub pool_id:       Pubkey,
    pub mint:          Pubkey,
    pub threshold_up:  u16,
    pub threshold_down: u16,
    pub fee_min:       u16,
    pub fee_max:       u16,
}

#[event]
pub struct AssetRemovedByGovernance {
    pub pool_id: Pubkey,
    pub mint:    Pubkey,
}

#[event]
pub struct FeeRangeUpdatedByGovernance {
    pub pool_id: Pubkey,
    pub mint:    Pubkey,
    pub old_min: u16,
    pub old_max: u16,
    pub new_min: u16,
    pub new_max: u16,
}

// ═══════════════════════════════════════════════════
// GOVERNANCE UPDATE MAX PCT
// Mirrors the Pool's max_pct_min/max_pct_max update onto
// AssetInfo so the Routing program can read current concentration
// limits directly from InfoPoolAccount without cross-program reads.
// Must be called atomically alongside Pool::governance_update_max_pct.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct GovernanceUpdateMaxPct<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_update_max_pct(
    ctx: Context<GovernanceUpdateMaxPct>,
    mint:    Pubkey,
    new_min: u8,
    new_max: u8,
) -> Result<()> {
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id,
        InfoPoolError::NotGovernance
    );
    require!(new_min < new_max && new_max <= 100, InfoPoolError::InvalidThreshold);

    let info_pool = &mut ctx.accounts.info_pool;

    let asset = info_pool.assets
        .iter_mut()
        .find(|a| a.mint == mint)
        .ok_or(InfoPoolError::AssetNotFound)?;

    let old_min = asset.max_pct_min;
    let old_max = asset.max_pct_max;

    asset.max_pct_min = new_min;
    asset.max_pct_max = new_max;

    emit!(MaxPctUpdatedByGovernance {
        pool_id: info_pool.pool_id,
        mint,
        old_min,
        old_max,
        new_min,
        new_max,
    });

    Ok(())
}

#[event]
pub struct MaxPctUpdatedByGovernance {
    pub pool_id: Pubkey,
    pub mint:    Pubkey,
    pub old_min: u8,
    pub old_max: u8,
    pub new_min: u8,
    pub new_max: u8,
}

// ═══════════════════════════════════════════════════
// GOVERNANCE SET ALLOWANCE
// Mirrors the Pool's allowance update onto AssetInfo so the
// Routing program can filter tradeable pairs from a single
// InfoPoolAccount read without touching the Pool program.
// Must be called atomically alongside Pool::governance_set_allowance.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(asset_mint: Pubkey)]
pub struct GovernanceSetAllowance<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_set_allowance(
    ctx: Context<GovernanceSetAllowance>,
    asset_mint:  Pubkey,
    target_mint: Pubkey,
    allowed:     bool,
) -> Result<()> {
    let gov_id: Pubkey = GOVERNANCE_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotGovernance))?;
    require!(
        *ctx.accounts.governance_authority.to_account_info().owner == gov_id,
        InfoPoolError::NotGovernance
    );

    let info_pool = &mut ctx.accounts.info_pool;

    let asset = info_pool.assets
        .iter_mut()
        .find(|a| a.mint == asset_mint)
        .ok_or(InfoPoolError::AssetNotFound)?;

    if allowed {
        if !asset.allowed.contains(&target_mint) {
            require!(
                asset.allowed.len() < AssetInfo::MAX_ALLOWED,
                InfoPoolError::TooManyAssets
            );
            asset.allowed.push(target_mint);
        }
    } else {
        asset.allowed.retain(|&x| x != target_mint);
    }

    emit!(AllowanceUpdatedByGovernance {
        pool_id:     info_pool.pool_id,
        asset_mint,
        target_mint,
        allowed,
    });

    Ok(())
}

#[event]
pub struct AllowanceUpdatedByGovernance {
    pub pool_id:     Pubkey,
    pub asset_mint:  Pubkey,
    pub target_mint: Pubkey,
    pub allowed:     bool,
      }
