use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::state::*;
use crate::constants::*;
use crate::errors::PoolError;

#[derive(Accounts)]
pub struct Swap<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
        constraint = pool.is_active @ PoolError::PoolNotActive
    )]
    pub pool: Account<'info, PoolAccount>,

    /// Outgoing asset account (asset leaving pool — e.g. BTC)
    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), asset_out.mint.as_ref()],
        bump = asset_out.bump,
    )]
    pub asset_out: Account<'info, AssetAccount>,

    /// Incoming asset account (asset entering pool — e.g. ETH)
    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), asset_in.mint.as_ref()],
        bump = asset_in.bump,
    )]
    pub asset_in: Account<'info, AssetAccount>,

    /// Pool vault for outgoing asset (BTC).
    /// Must be owned by this pool PDA and hold the correct mint.
    #[account(
        mut,
        constraint = pool_vault_out.owner == pool.key() @ PoolError::Unauthorized,
        constraint = pool_vault_out.mint  == asset_out.mint @ PoolError::AssetNotAllowed,
    )]
    pub pool_vault_out: Account<'info, TokenAccount>,

    /// Pool vault for incoming asset (ETH).
    /// Must be owned by this pool PDA and hold the correct mint.
    #[account(
        mut,
        constraint = pool_vault_in.owner == pool.key() @ PoolError::Unauthorized,
        constraint = pool_vault_in.mint  == asset_in.mint @ PoolError::AssetNotAllowed,
    )]
    pub pool_vault_in: Account<'info, TokenAccount>,

    /// User's token account for outgoing asset (receives BTC)
    #[account(mut)]
    pub user_token_out: Account<'info, TokenAccount>,

    /// User's token account for incoming asset (sends ETH)
    #[account(mut)]
    pub user_token_in: Account<'info, TokenAccount>,

    /// User authority
    pub user: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

/// Oracle-rate based swap.
///
/// `amount_in`      — tokens user is sending (e.g. ETH amount)
/// `min_amount_out` — minimum tokens user will accept (slippage guard)
///
/// Oracle rates are read from `asset_in.oracle_price` and `asset_out.oracle_price`,
/// which are pushed by InfoPool via `update_oracle_price` CPI after each Pyth tick.
/// User-supplied rates are NOT accepted — this eliminates the price-manipulation vector.
///
/// Pricing: amount_out = (amount_in × rate_in) / rate_out
/// Fee is applied to the outgoing asset only (from InfoPool's pushed `current_fee`).
///
/// IMPORTANT: `oracle_price` starts at 0 on asset creation. InfoPool must call
/// `update_oracle_price` before this swap can execute for the first time.
pub fn handler(
    ctx: Context<Swap>,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<()> {
    let pool      = &mut ctx.accounts.pool;
    let asset_out = &mut ctx.accounts.asset_out;
    let asset_in  = &mut ctx.accounts.asset_in;

    // ── STEP 1: CHECK ASSET INTERACTION ──────────
    // BTC (asset_out) must allow ETH (asset_in)
    let interaction_allowed = asset_out
        .allowed
        .contains(&asset_in.mint);

    require!(interaction_allowed, PoolError::InteractionNotAllowed);

    // ── STEP 2: CHECK INFLOW BLOCKED ─────────────
    require!(!asset_in.is_blocked, PoolError::InflowBlocked);

    // ── STEP 3: (concentration guard delegated to InfoPool threshold engine)
    // InfoPool crank sets is_blocked = true when price breaches threshold_up /
    // threshold_down; STEP 2 already rejects the swap at that point.

    // ── STEP 4: READ ORACLE RATES FROM ASSET STATE ──
    // Rates are pushed by InfoPool via update_oracle_price CPI after each Pyth tick.
    // User-supplied rates are NOT accepted — eliminates the price-manipulation vector.
    let rate_in  = asset_in.oracle_price;
    let rate_out = asset_out.oracle_price;
    require!(rate_in  > 0, PoolError::OraclePriceNotSet);
    require!(rate_out > 0, PoolError::OraclePriceNotSet);
    require!(asset_out.amount > 0, PoolError::InsufficientLiquidity);

    // ── STEP 5: CALCULATE AMOUNT OUT (ORACLE RATE) ──
    // Pricing is oracle-driven, not reserve-curve driven.
    // amount_out = (amount_in × rate_in) / rate_out
    let amount_out_before_fee = (amount_in as u128)
        .checked_mul(rate_in as u128)
        .ok_or(PoolError::MathOverflow)?
        .checked_div(rate_out as u128)
        .ok_or(PoolError::MathOverflow)? as u64;

    // ── STEP 6: APPLY OUTGOING FEE ONLY ──────────
    // Dynamic fee from InfoPool on the outgoing asset only.
    // Fee is deducted from amount_out; it remains inside the pool vault.
    let fee_bps    = asset_out.current_fee as u64;
    let fee_amount = amount_out_before_fee
        .checked_mul(fee_bps)
        .ok_or(PoolError::MathOverflow)?
        .checked_div(BPS_DENOMINATOR)
        .ok_or(PoolError::MathOverflow)?;

    let amount_out = amount_out_before_fee
        .checked_sub(fee_amount)
        .ok_or(PoolError::MathOverflow)?;

    // Pool must have enough balance to pay out the post-fee amount.
    // The fee (fee_amount) stays in the vault, so only amount_out actually
    // leaves. Checking against amount_out_before_fee would falsely reject
    // swaps where the pool has exactly enough after the fee deduction.
    require!(
        asset_out.amount >= amount_out,
        PoolError::InsufficientLiquidity
    );

    // ── STEP 7: SLIPPAGE CHECK ────────────────────
    require!(amount_out >= min_amount_out, PoolError::SlippageExceeded);

    // ── STEP 8: EXECUTE TOKEN TRANSFERS ──────────

    // Transfer asset_in from user to pool vault
    let cpi_accounts_in = Transfer {
        from:      ctx.accounts.user_token_in.to_account_info(),
        to:        ctx.accounts.pool_vault_in.to_account_info(),
        authority: ctx.accounts.user.to_account_info(),
    };
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts_in,
        ),
        amount_in,
    )?;

    // Transfer asset_out from pool vault to user
    let pool_key  = pool.key();
    let bump      = pool.bump;
    let seeds     = &[
        POOL_SEED,
        pool.owner.as_ref(),
        &[bump],
    ];
    let signer = &[&seeds[..]];

    let cpi_accounts_out = Transfer {
        from:      ctx.accounts.pool_vault_out.to_account_info(),
        to:        ctx.accounts.user_token_out.to_account_info(),
        authority: pool.to_account_info(),
    };
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts_out,
            signer,
        ),
        amount_out,
    )?;

    // ── STEP 9: UPDATE STATE ──────────────────────
    asset_in.amount  = asset_in.amount
        .checked_add(amount_in)
        .ok_or(PoolError::MathOverflow)?;

    asset_out.amount = asset_out.amount
        .checked_sub(amount_out)
        .ok_or(PoolError::MathOverflow)?;

    // NOTE: total_value is NOT updated on swap.
    // Updating it here with raw token amounts mixes different token units
    // (e.g. ETH wei vs BTC satoshi), making the value meaningless.
    // Concentration checks use InfoPool's pool_size instead.
    // total_value is only updated on deposit/withdraw, where a single asset
    // is involved so units are consistent.

    // Update pool weight (fee earned = net value retained in pool)
    pool.pool_weight = pool.pool_weight
        .checked_add(fee_amount)
        .ok_or(PoolError::MathOverflow)?;

    // ── STEP 10: UPDATE FEE ACCUMULATOR ──────────
    // Single-asset independent model: only LPs who deposited asset_out earn
    // fees when it is swapped out.  fees_per_share is a monotonic accumulator
    // scaled by FEE_SCALE so sub-lamport precision is maintained.
    // If no LP has deposited yet (total_deposited == 0) the fee stays in the
    // vault and is claimable by whoever deposits next via the accumulator.
    if asset_out.total_deposited > 0 && fee_amount > 0 {
        let fps_inc = (fee_amount as u128)
            .checked_mul(FEE_SCALE as u128)
            .unwrap_or(0)
            .checked_div(asset_out.total_deposited as u128)
            .unwrap_or(0) as u64;
        asset_out.fees_per_share = asset_out.fees_per_share.saturating_add(fps_inc);
    }

    emit!(SwapExecuted {
        pool:         pool_key,
        asset_in:     asset_in.mint,
        asset_out:    asset_out.mint,
        amount_in,
        amount_out,
        fee_amount,
        fee_bps:      fee_bps as u16,
        rate_in,
        rate_out,
    });

    Ok(())
}

#[event]
pub struct SwapExecuted {
    pub pool:       Pubkey,
    pub asset_in:   Pubkey,
    pub asset_out:  Pubkey,
    pub amount_in:  u64,
    pub amount_out: u64,
    pub fee_amount: u64,
    pub fee_bps:    u16,
    /// Oracle price of asset_in — sourced from InfoPool, not user-supplied
    pub rate_in:    u64,
    /// Oracle price of asset_out — sourced from InfoPool, not user-supplied
    pub rate_out:   u64,
  }
  
