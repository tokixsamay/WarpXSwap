use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::InfoPoolError;

// ═══════════════════════════════════════════════════
// GET POOL STATE — Full state for routing
// Only callable via CPI from authorized programs
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct GetPoolState<'info> {
    #[account(
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    /// Must be Pool Program or Routing Program PDA
    pub caller: Signer<'info>,
}

pub fn handler_pool_state(
    ctx: Context<GetPoolState>,
) -> Result<PoolStateResponse> {
    // Verify caller is a PDA owned by the Pool or Routing program
    let pool_id: Pubkey = POOL_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotPoolProgram))?;
    let routing_id: Pubkey = crate::constants::ROUTING_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotPoolProgram))?;
    let caller_owner = *ctx.accounts.caller.to_account_info().owner;
    require!(
        caller_owner == pool_id || caller_owner == routing_id,
        InfoPoolError::NotPoolProgram
    );

    let info_pool = &ctx.accounts.info_pool;

    let assets: Vec<AssetSummary> = info_pool.assets
        .iter()
        .map(|a| AssetSummary {
            mint:            a.mint,
            current_pct:     a.current_pct,
            current_fee:     a.current_fee,
            is_blocked:      a.is_blocked,
            threshold_state: a.threshold_state.clone(),
        })
        .collect();

    Ok(PoolStateResponse {
        pool_id:     info_pool.pool_id,
        pool_size:   info_pool.pool_size,
        pool_weight: info_pool.pool_weight,
        assets,
    })
}

// ═══════════════════════════════════════════════════
// GET ASSET FEE — Single asset fee query
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct GetAssetFee<'info> {
    #[account(
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    pub caller: Signer<'info>,
}

pub fn handler_asset_fee(
    ctx: Context<GetAssetFee>,
    mint: Pubkey,
) -> Result<u16> {
    let pool_id: Pubkey = POOL_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotPoolProgram))?;
    let routing_id: Pubkey = crate::constants::ROUTING_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotPoolProgram))?;
    let caller_owner = *ctx.accounts.caller.to_account_info().owner;
    require!(
        caller_owner == pool_id || caller_owner == routing_id,
        InfoPoolError::NotPoolProgram
    );

    let info_pool = &ctx.accounts.info_pool;

    let asset = info_pool.assets
        .iter()
        .find(|a| a.mint == mint)
        .ok_or(InfoPoolError::AssetNotFound)?;

    Ok(asset.current_fee)
}

// ═══════════════════════════════════════════════════
// GET THRESHOLD STATE — For routing priority
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct GetThresholdState<'info> {
    #[account(
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    pub caller: Signer<'info>,
}

pub fn handler_threshold_state(
    ctx: Context<GetThresholdState>,
    mint: Pubkey,
) -> Result<ThresholdStateResponse> {
    let pool_id: Pubkey = POOL_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotPoolProgram))?;
    let routing_id: Pubkey = crate::constants::ROUTING_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotPoolProgram))?;
    let caller_owner = *ctx.accounts.caller.to_account_info().owner;
    require!(
        caller_owner == pool_id || caller_owner == routing_id,
        InfoPoolError::NotPoolProgram
    );

    let info_pool = &ctx.accounts.info_pool;

    let asset = info_pool.assets
        .iter()
        .find(|a| a.mint == mint)
        .ok_or(InfoPoolError::AssetNotFound)?;

    Ok(ThresholdStateResponse {
        mint,
        state:        asset.threshold_state.clone(),
        current_fee:  asset.current_fee,
        is_blocked:   asset.is_blocked,
        layer_status: asset.layer_status.clone(),
    })
      }
