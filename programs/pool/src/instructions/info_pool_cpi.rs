use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::PoolError;

// ═══════════════════════════════════════════════════
// UPDATE FEE — Called by Info Pool Program via CPI
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey, new_fee: u16)]
pub struct UpdateFee<'info> {
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

    /// Must be the info_pool PDA (owned by the Info Pool program).
    /// When InfoPool calls this via CPI with invoke_signed, the PDA
    /// is the signer and its owner == INFO_POOL_PROGRAM_ID.
    pub info_pool_authority: Signer<'info>,
}

pub fn handler_update_fee(
    ctx: Context<UpdateFee>,
    mint: Pubkey,
    new_fee: u16,
) -> Result<()> {
    // Verify the signer is a PDA owned by the Info Pool program.
    // An account's owner is the program that created it; Info Pool PDAs
    // are always owned by INFO_POOL_PROGRAM_ID.
    let expected_program: Pubkey = INFO_POOL_PROGRAM_ID
        .parse()
        .map_err(|_| error!(PoolError::NotInfoPool))?;
    require!(
        *ctx.accounts.info_pool_authority.to_account_info().owner == expected_program,
        PoolError::NotInfoPool
    );

    let asset = &mut ctx.accounts.asset;

    // Fee must be within LP-set bounds
    require!(
        new_fee >= asset.fee_min && new_fee <= asset.fee_max,
        PoolError::FeeOutOfRange
    );

    let old_fee = asset.current_fee;
    asset.current_fee = new_fee;

    emit!(FeeUpdated {
        pool:    ctx.accounts.pool.key(),
        mint,
        old_fee,
        new_fee,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// BLOCK INFLOW — Called by Info Pool when threshold exceeded
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct BlockInflow<'info> {
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

    pub info_pool_authority: Signer<'info>,
}

pub fn handler_block_inflow(
    ctx: Context<BlockInflow>,
    mint: Pubkey,
) -> Result<()> {
    let expected_program: Pubkey = INFO_POOL_PROGRAM_ID
        .parse()
        .map_err(|_| error!(PoolError::NotInfoPool))?;
    require!(
        *ctx.accounts.info_pool_authority.to_account_info().owner == expected_program,
        PoolError::NotInfoPool
    );

    let asset = &mut ctx.accounts.asset;
    // Only flip the blocked flag here.
    // threshold_state is managed exclusively by InfoPool's run_threshold_check
    // (which knows whether it's ExceededUp or ExceededDown).
    // Overwriting it here with a hardcoded variant would corrupt the state.
    asset.is_blocked = true;

    emit!(InflowBlocked {
        pool: ctx.accounts.pool.key(),
        mint,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// UNBLOCK INFLOW — Called by Info Pool when threshold recovers
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct UnblockInflow<'info> {
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

    pub info_pool_authority: Signer<'info>,
}

pub fn handler_unblock_inflow(
    ctx: Context<UnblockInflow>,
    mint: Pubkey,
) -> Result<()> {
    let expected_program: Pubkey = INFO_POOL_PROGRAM_ID
        .parse()
        .map_err(|_| error!(PoolError::NotInfoPool))?;
    require!(
        *ctx.accounts.info_pool_authority.to_account_info().owner == expected_program,
        PoolError::NotInfoPool
    );

    let asset = &mut ctx.accounts.asset;
    asset.is_blocked      = false;
    asset.threshold_state = ThresholdState::Neutral;

    emit!(InflowUnblocked {
        pool: ctx.accounts.pool.key(),
        mint,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// UPDATE ORACLE PRICE — Called by Info Pool via CPI
// Pushes the latest Pyth spot price into Pool's AssetAccount.oracle_price
// so Pool's swap can read oracle prices without a circular CPI dependency.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey, price: u64)]
pub struct UpdateOraclePrice<'info> {
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

    /// Must be the info_pool PDA (owned by the Info Pool program).
    pub info_pool_authority: Signer<'info>,
}

pub fn handler_update_oracle_price(
    ctx: Context<UpdateOraclePrice>,
    mint: Pubkey,
    price: u64,
) -> Result<()> {
    let expected_program: Pubkey = INFO_POOL_PROGRAM_ID
        .parse()
        .map_err(|_| error!(PoolError::NotInfoPool))?;
    require!(
        *ctx.accounts.info_pool_authority.to_account_info().owner == expected_program,
        PoolError::NotInfoPool
    );
    require!(price > 0, PoolError::InvalidRate);

    ctx.accounts.asset.oracle_price = price;

    emit!(OraclePriceUpdated {
        pool:  ctx.accounts.pool.key(),
        mint,
        price,
    });

    Ok(())
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct FeeUpdated {
    pub pool:    Pubkey,
    pub mint:    Pubkey,
    pub old_fee: u16,
    pub new_fee: u16,
}

#[event]
pub struct InflowBlocked {
    pub pool: Pubkey,
    pub mint: Pubkey,
}

#[event]
pub struct InflowUnblocked {
    pub pool: Pubkey,
    pub mint: Pubkey,
}

#[event]
pub struct OraclePriceUpdated {
    pub pool:  Pubkey,
    pub mint:  Pubkey,
    pub price: u64,
}
