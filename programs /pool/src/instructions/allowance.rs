use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::PoolError;

// ═══════════════════════════════════════════════════
// SET ALLOWANCE
// Asset A allows/disallows interaction with Asset B
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(target_mint: Pubkey, allowed: bool)]
pub struct SetAllowance<'info> {
    #[account(
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, PoolAccount>,

    /// Asset whose allowance list we are updating
    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), asset.mint.as_ref()],
        bump = asset.bump,
    )]
    pub asset: Account<'info, AssetAccount>,

    #[account(
        constraint = authority.key() == pool.owner @ PoolError::Unauthorized
    )]
    pub authority: Signer<'info>,
}

pub fn handler_set_allowance(
    ctx: Context<SetAllowance>,
    target_mint: Pubkey,
    allowed: bool,
) -> Result<()> {
    let asset = &mut ctx.accounts.asset;

    if allowed {
        // Add to allowed list if not already present
        if !asset.allowed.contains(&target_mint) {
            require!(
                asset.allowed.len() < AssetAccount::MAX_ALLOWED,
                PoolError::TooManyAllowed
            );
            asset.allowed.push(target_mint);
        }
    } else {
        // Remove from allowed list
        asset.allowed.retain(|&x| x != target_mint);
    }

    emit!(AllowanceUpdated {
        pool:        ctx.accounts.pool.key(),
        asset:       asset.mint,
        target_mint,
        allowed,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// REMOVE ASSET
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct RemoveAsset<'info> {
    #[account(
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        mut,
        close = authority,
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

    #[account(
        mut,
        constraint = authority.key() == pool.owner @ PoolError::Unauthorized
    )]
    pub authority: Signer<'info>,
}

pub fn handler_remove_asset(
    ctx: Context<RemoveAsset>,
) -> Result<()> {
    let mint = ctx.accounts.asset.mint;

    emit!(AssetRemoved {
        pool: ctx.accounts.pool.key(),
        mint,
    });

    // Account closed via `close = authority` constraint
    Ok(())
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct AllowanceUpdated {
    pub pool:        Pubkey,
    pub asset:       Pubkey,
    pub target_mint: Pubkey,
    pub allowed:     bool,
}

#[event]
pub struct AssetRemoved {
    pub pool: Pubkey,
    pub mint: Pubkey,
  }
  
