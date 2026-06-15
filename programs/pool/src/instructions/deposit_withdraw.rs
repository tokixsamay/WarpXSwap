use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::state::*;
use crate::constants::*;
use crate::errors::PoolError;

// ═══════════════════════════════════════════════════
// DEPOSIT — LP adds liquidity
//
// On every deposit we record (or update) a per-user
// LpDepositAccount PDA so PublicExit can enforce that
// an LP cannot withdraw more than they contributed.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
        constraint = pool.is_active @ PoolError::PoolNotActive
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), asset.mint.as_ref()],
        bump = asset.bump,
    )]
    pub asset: Account<'info, AssetAccount>,

    /// Pool token vault for this asset.
    /// Must be owned by this pool PDA and match the asset's mint.
    #[account(
        mut,
        constraint = pool_vault.owner == pool.key() @ PoolError::Unauthorized,
        constraint = pool_vault.mint  == asset.mint @ PoolError::AssetNotAllowed,
    )]
    pub pool_vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub user_token: Account<'info, TokenAccount>,

    /// Per-(pool, asset, user) deposit tracker.
    /// Initialised on the first deposit; updated on every subsequent one.
    /// Prevents any LP from exiting more than their net contribution.
    #[account(
        init_if_needed,
        payer  = user,
        space  = LpDepositAccount::LEN,
        seeds  = [
            LP_DEPOSIT_SEED,
            pool.key().as_ref(),
            asset.mint.as_ref(),
            user.key().as_ref(),
        ],
        bump,
    )]
    pub lp_deposit: Account<'info, LpDepositAccount>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub token_program:  Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub fn handler_deposit(
    ctx: Context<Deposit>,
    amount: u64,
) -> Result<()> {
    let pool       = &mut ctx.accounts.pool;
    let asset      = &mut ctx.accounts.asset;
    let lp_deposit = &mut ctx.accounts.lp_deposit;

    require!(amount > 0, PoolError::InsufficientBalance);

    // NOTE: LP deposits are NOT subject to the inflow block.
    // Only external swaps (swap.rs) check is_blocked.
    // This allows LPs to always add liquidity even during threshold events,
    // which is intentional — the LP is the pool operator, not an external trader.

    // Transfer tokens from user to pool vault
    let cpi_accounts = Transfer {
        from:      ctx.accounts.user_token.to_account_info(),
        to:        ctx.accounts.pool_vault.to_account_info(),
        authority: ctx.accounts.user.to_account_info(),
    };
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
        ),
        amount,
    )?;

    // ── UPDATE ASSET & POOL STATE ─────────────────
    asset.amount = asset.amount
        .checked_add(amount)
        .ok_or(PoolError::MathOverflow)?;

    pool.total_value = pool.total_value
        .checked_add(amount)
        .ok_or(PoolError::MathOverflow)?;
    pool.pool_weight = pool.pool_weight
        .checked_add(amount)
        .ok_or(PoolError::MathOverflow)?;

    // ── TRACK LP CONTRIBUTION ─────────────────────
    // Single-asset independent model: each LP earns fees only on the asset
    // they deposited, proportional to their share of total_deposited.
    //
    // First deposit: initialise PDA + set fee_debt = current accumulator
    //   so the LP does not claim fees earned before their arrival.
    // Re-deposit: settle accrued fees into pending_fees BEFORE resetting
    //   fee_debt, so existing earnings are not lost when debt is updated.
    let is_first_deposit = lp_deposit.pool == Pubkey::default();
    if is_first_deposit {
        lp_deposit.pool         = pool.key();
        lp_deposit.asset        = asset.mint;
        lp_deposit.depositor    = ctx.accounts.user.key();
        lp_deposit.bump         = ctx.bumps.lp_deposit;
        lp_deposit.fee_debt     = asset.fees_per_share;
        lp_deposit.pending_fees = 0;
    } else {
        // Settle fees accrued on the existing position so they are preserved
        // when fee_debt is reset to the current accumulator value.
        let fps_delta = asset.fees_per_share.saturating_sub(lp_deposit.fee_debt);
        let accrued   = (lp_deposit.amount as u128)
            .checked_mul(fps_delta as u128)
            .unwrap_or(0)
            .checked_div(FEE_SCALE as u128)
            .unwrap_or(0) as u64;
        lp_deposit.pending_fees = lp_deposit.pending_fees
            .checked_add(accrued)
            .ok_or(PoolError::MathOverflow)?;
        lp_deposit.fee_debt = asset.fees_per_share;
    }
    lp_deposit.amount = lp_deposit.amount
        .checked_add(amount)
        .ok_or(PoolError::MathOverflow)?;

    // ── TRACK TOTAL PRINCIPAL PER ASSET ──────────
    asset.total_deposited = asset.total_deposited
        .checked_add(amount)
        .ok_or(PoolError::MathOverflow)?;

    emit!(Deposited {
        pool:   pool.key(),
        mint:   asset.mint,
        amount,
        user:   ctx.accounts.user.key(),
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// WITHDRAW BASE — Private pool: exit in base asset
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct WithdrawBase<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
        constraint = pool.pool_type == PoolType::Private @ PoolError::PoolTypeMismatch
    )]
    pub pool: Account<'info, PoolAccount>,

    /// Base asset account
    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), pool.base_asset.as_ref()],
        bump = base_asset.bump,
    )]
    pub base_asset: Account<'info, AssetAccount>,

    /// Pool token vault for the base asset.
    /// Must be owned by this pool PDA and match the base asset's mint.
    #[account(
        mut,
        constraint = pool_vault_base.owner == pool.key() @ PoolError::Unauthorized,
        constraint = pool_vault_base.mint  == base_asset.mint @ PoolError::AssetNotAllowed,
    )]
    pub pool_vault_base: Account<'info, TokenAccount>,

    #[account(mut)]
    pub user_token_base: Account<'info, TokenAccount>,

    /// LP must be pool owner for private pool
    #[account(
        constraint = authority.key() == pool.owner @ PoolError::Unauthorized
    )]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handler_withdraw_base(
    ctx: Context<WithdrawBase>,
    amount: u64,
) -> Result<()> {
    let pool       = &mut ctx.accounts.pool;
    let base_asset = &mut ctx.accounts.base_asset;

    require!(
        base_asset.amount >= amount,
        PoolError::InsufficientBalance
    );

    // Transfer base asset from pool to user
    let bump  = pool.bump;
    let seeds = &[
        POOL_SEED,
        pool.owner.as_ref(),
        &[bump],
    ];
    let signer = &[&seeds[..]];

    let cpi_accounts = Transfer {
        from:      ctx.accounts.pool_vault_base.to_account_info(),
        to:        ctx.accounts.user_token_base.to_account_info(),
        authority: pool.to_account_info(),
    };
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer,
        ),
        amount,
    )?;

    base_asset.amount = base_asset.amount
        .checked_sub(amount)
        .ok_or(PoolError::MathOverflow)?;

    pool.total_value = pool.total_value
        .checked_sub(amount)
        .ok_or(PoolError::MathOverflow)?;
    pool.pool_weight = pool.pool_weight
        .checked_sub(amount)
        .ok_or(PoolError::WeightError)?;

    emit!(WithdrewBase {
        pool:   pool.key(),
        amount,
        user:   ctx.accounts.authority.key(),
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// WITHDRAW ALL — LP withdraws percentage of any asset
// Private pools ONLY (Public pool LPs use public_exit)
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct WithdrawAll<'info> {
    // Private pools ONLY.
    // Public pool LPs must use public_exit, which enforces per-depositor limits.
    // Allowing the pool owner to call withdraw_all on a public pool would let
    // the founding member drain liquidity deposited by other LPs.
    #[account(
        mut,
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
        constraint = pool.is_active @ PoolError::PoolNotActive,
        constraint = pool.pool_type == PoolType::Private @ PoolError::PoolTypeMismatch
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), asset.mint.as_ref()],
        bump = asset.bump,
    )]
    pub asset: Account<'info, AssetAccount>,

    /// Pool token vault for this asset (WithdrawAll).
    /// Must be owned by this pool PDA and match the asset's mint.
    #[account(
        mut,
        constraint = pool_vault.owner == pool.key() @ PoolError::Unauthorized,
        constraint = pool_vault.mint  == asset.mint @ PoolError::AssetNotAllowed,
    )]
    pub pool_vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub user_token: Account<'info, TokenAccount>,

    #[account(
        constraint = authority.key() == pool.owner @ PoolError::Unauthorized
    )]
    pub authority: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handler_withdraw_all(
    ctx: Context<WithdrawAll>,
    percentage: u8,
) -> Result<()> {
    require!(
        percentage > 0 && percentage <= 100,
        PoolError::InvalidPercentage
    );

    let pool  = &mut ctx.accounts.pool;
    let asset = &mut ctx.accounts.asset;

    let withdraw_amount = (asset.amount as u128)
        .checked_mul(percentage as u128)
        .ok_or(PoolError::MathOverflow)?
        .checked_div(100)
        .ok_or(PoolError::MathOverflow)? as u64;

    require!(withdraw_amount > 0, PoolError::InsufficientBalance);
    require!(asset.amount >= withdraw_amount, PoolError::InsufficientBalance);

    let bump   = pool.bump;
    let seeds  = &[POOL_SEED, pool.owner.as_ref(), &[bump]];
    let signer = &[&seeds[..]];

    let cpi_accounts = Transfer {
        from:      ctx.accounts.pool_vault.to_account_info(),
        to:        ctx.accounts.user_token.to_account_info(),
        authority: pool.to_account_info(),
    };
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer,
        ),
        withdraw_amount,
    )?;

    asset.amount = asset.amount
        .checked_sub(withdraw_amount)
        .ok_or(PoolError::MathOverflow)?;

    pool.total_value = pool.total_value
        .checked_sub(withdraw_amount)
        .ok_or(PoolError::MathOverflow)?;
    pool.pool_weight = pool.pool_weight
        .checked_sub(withdraw_amount)
        .ok_or(PoolError::WeightError)?;

    emit!(WithdrewAll {
        pool:       pool.key(),
        mint:       asset.mint,
        amount:     withdraw_amount,
        percentage,
        user:       ctx.accounts.authority.key(),
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// PUBLIC EXIT — Any depositor can exit a public pool
//
// Gated by the LpDepositAccount PDA: an LP can only
// withdraw up to their net contribution for this asset.
// This prevents the rug vector where one user drains
// liquidity deposited by another.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct PublicExit<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
        constraint = pool.is_active @ PoolError::PoolNotActive,
        constraint = pool.pool_type == PoolType::Public @ PoolError::PoolTypeMismatch
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), asset.mint.as_ref()],
        bump = asset.bump,
    )]
    pub asset: Account<'info, AssetAccount>,

    /// Pool token vault for this asset (PublicExit).
    /// Must be owned by this pool PDA and match the asset's mint.
    #[account(
        mut,
        constraint = pool_vault.owner == pool.key() @ PoolError::Unauthorized,
        constraint = pool_vault.mint  == asset.mint @ PoolError::AssetNotAllowed,
    )]
    pub pool_vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub user_token: Account<'info, TokenAccount>,

    /// Per-(pool, asset, user) deposit tracker.
    /// Enforces that this LP can only exit what they deposited.
    /// Seeds tie the record to the specific user — no cross-user access.
    #[account(
        mut,
        seeds = [
            LP_DEPOSIT_SEED,
            pool.key().as_ref(),
            asset.mint.as_ref(),
            user.key().as_ref(),
        ],
        bump = lp_deposit.bump,
    )]
    pub lp_deposit: Account<'info, LpDepositAccount>,

    pub user: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handler_public_exit(
    ctx: Context<PublicExit>,
    amount: u64,
) -> Result<()> {
    let pool       = &mut ctx.accounts.pool;
    let asset      = &mut ctx.accounts.asset;
    let lp_deposit = &mut ctx.accounts.lp_deposit;

    require!(amount > 0, PoolError::InsufficientBalance);

    // ── LP SHARE GATE ─────────────────────────────
    // Verify this user has enough principal to cover the withdrawal.
    require!(
        lp_deposit.amount >= amount,
        PoolError::ExceedsDeposit
    );

    // ── COMPUTE FEE SHARE ─────────────────────────
    // Single-asset model: LP earns fees proportional to their share of
    // total_deposited for this specific asset, from the moment they deposited.
    //
    // claimable = pending_fees + amount_exiting × (fps − fee_debt) / FEE_SCALE
    //
    // We only charge the exiting fraction of the accumulated delta — the
    // remainder stays credited for whatever principal the LP keeps in the pool.
    let fps_delta = asset.fees_per_share.saturating_sub(lp_deposit.fee_debt);

    // Accrued fees on the FULL current position (we'll apportion by exit ratio)
    let full_accrued = (lp_deposit.amount as u128)
        .checked_mul(fps_delta as u128)
        .unwrap_or(0)
        .checked_div(FEE_SCALE as u128)
        .unwrap_or(0) as u64;

    // Pro-rate: if LP exits fraction (amount / lp_deposit.amount) of principal,
    // they take the same fraction of accrued fees + pending_fees.
    let exit_fee_accrued = (full_accrued as u128)
        .checked_mul(amount as u128)
        .unwrap_or(0)
        .checked_div(lp_deposit.amount as u128)
        .unwrap_or(0) as u64;

    let exit_pending = (lp_deposit.pending_fees as u128)
        .checked_mul(amount as u128)
        .unwrap_or(0)
        .checked_div(lp_deposit.amount as u128)
        .unwrap_or(0) as u64;

    let fee_share = exit_fee_accrued
        .checked_add(exit_pending)
        .ok_or(PoolError::MathOverflow)?;

    // ── LIQUIDITY CHECK ───────────────────────────
    let total_out = amount
        .checked_add(fee_share)
        .ok_or(PoolError::MathOverflow)?;
    require!(asset.amount >= total_out, PoolError::InsufficientBalance);

    // ── TRANSFER principal + fee_share ────────────
    let bump   = pool.bump;
    let seeds  = &[POOL_SEED, pool.owner.as_ref(), &[bump]];
    let signer = &[&seeds[..]];

    let cpi_accounts = Transfer {
        from:      ctx.accounts.pool_vault.to_account_info(),
        to:        ctx.accounts.user_token.to_account_info(),
        authority: pool.to_account_info(),
    };
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer,
        ),
        total_out,
    )?;

    // ── UPDATE STATE ──────────────────────────────
    // asset.amount tracks vault balance including accumulated fees.
    asset.amount = asset.amount
        .checked_sub(total_out)
        .ok_or(PoolError::MathOverflow)?;

    // total_deposited tracks principal only — reduce by the principal exited.
    asset.total_deposited = asset.total_deposited
        .checked_sub(amount)
        .ok_or(PoolError::MathOverflow)?;

    // pool.total_value tracks deposited principal only (swap fees are NOT added
    // to total_value — they go to pool_weight only via swap.rs STEP 9).
    // So only the exiting principal (amount) is subtracted from total_value.
    // The fee_share component comes out of pool_weight, not total_value.
    pool.total_value = pool.total_value
        .checked_sub(amount)
        .ok_or(PoolError::MathOverflow)?;
    pool.pool_weight = pool.pool_weight
        .checked_sub(total_out)
        .ok_or(PoolError::WeightError)?;

    // Reduce principal record; cannot double-exit.
    lp_deposit.amount = lp_deposit.amount
        .checked_sub(amount)
        .ok_or(PoolError::MathOverflow)?;

    // ── SAVE REMAINING FEE TO pending_fees BEFORE RESETTING fee_debt ──
    // Critical: the remaining (non-exiting) fraction of the accrued delta
    // must be saved into pending_fees NOW.  If we reset fee_debt without
    // saving, the next fps_delta computation yields 0 and those fees are
    // permanently lost.
    //
    //   remaining_accrued = full_accrued × (remaining_principal / old_amount)
    //                     = full_accrued - exit_fee_accrued
    //   remaining_pending = pending_fees - exit_pending (already paid)
    //   new pending_fees  = remaining_pending + remaining_accrued
    let remaining_accrued = full_accrued.saturating_sub(exit_fee_accrued);
    let remaining_pending  = lp_deposit.pending_fees
        .checked_sub(exit_pending)
        .ok_or(PoolError::MathOverflow)?;
    lp_deposit.pending_fees = remaining_pending
        .checked_add(remaining_accrued)
        .ok_or(PoolError::MathOverflow)?;

    // Reset fee_debt — future accruals on the remaining position start
    // fresh from the current accumulator value.
    lp_deposit.fee_debt = asset.fees_per_share;

    emit!(PublicExited {
        pool:      pool.key(),
        mint:      asset.mint,
        principal: amount,
        fee_share,
        total_out,
        user:      ctx.accounts.user.key(),
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// COMPOUND FEES — LP re-deposits accrued fees back as principal
//
// Fees are ALREADY sitting in the pool vault — no token transfer needed.
// This is a pure accounting operation:
//   lp_deposit.amount    += claimable  (principal grows)
//   asset.total_deposited += claimable  (pool tracks larger depositor base)
//   pending_fees = 0, fee_debt = fps   (clean slate for future accruals)
//
// Result: LP's share of FUTURE fees grows proportionally (compounding effect).
// Public pools only.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct CompoundFees<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
        constraint = pool.is_active  @ PoolError::PoolNotActive,
        constraint = pool.pool_type == PoolType::Public @ PoolError::PoolTypeMismatch
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), asset.mint.as_ref()],
        bump = asset.bump,
    )]
    pub asset: Account<'info, AssetAccount>,

    /// Per-(pool, asset, user) deposit tracker.
    #[account(
        mut,
        seeds = [
            LP_DEPOSIT_SEED,
            pool.key().as_ref(),
            asset.mint.as_ref(),
            user.key().as_ref(),
        ],
        bump = lp_deposit.bump,
    )]
    pub lp_deposit: Account<'info, LpDepositAccount>,

    pub user: Signer<'info>,
}

pub fn handler_compound_fees(ctx: Context<CompoundFees>) -> Result<()> {
    let asset      = &mut ctx.accounts.asset;
    let lp_deposit = &mut ctx.accounts.lp_deposit;

    // ── COMPUTE CLAIMABLE FEES ────────────────────
    // Same formula as claim_fees — fees sitting in vault, not yet "principal".
    let fps_delta    = asset.fees_per_share.saturating_sub(lp_deposit.fee_debt);
    let full_accrued = (lp_deposit.amount as u128)
        .checked_mul(fps_delta as u128)
        .unwrap_or(0)
        .checked_div(FEE_SCALE as u128)
        .unwrap_or(0) as u64;
    let claimable = lp_deposit.pending_fees
        .checked_add(full_accrued)
        .ok_or(PoolError::MathOverflow)?;

    require!(claimable > 0, PoolError::InsufficientBalance);

    // ── COMPOUND: FEES → PRINCIPAL ────────────────
    // No token transfer — tokens stay in vault.
    // We simply reclassify them from "earned fees" to "deposited principal".
    lp_deposit.amount = lp_deposit.amount
        .checked_add(claimable)
        .ok_or(PoolError::MathOverflow)?;

    asset.total_deposited = asset.total_deposited
        .checked_add(claimable)
        .ok_or(PoolError::MathOverflow)?;

    // Reset fee tracking — fresh start from current accumulator.
    lp_deposit.pending_fees = 0;
    lp_deposit.fee_debt     = asset.fees_per_share;

    emit!(FeesCompounded {
        pool:               ctx.accounts.pool.key(),
        mint:               asset.mint,
        compounded_amount:  claimable,
        new_principal:      lp_deposit.amount,
        user:               ctx.accounts.user.key(),
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// CLAIM FEES — LP harvests fee yield without reducing principal
//
// Computes the full claimable fee share (pending_fees + newly accrued)
// and transfers it to the LP.  Principal position stays intact.
// fee_debt is reset so future accruals start from the current accumulator.
//
// Public pools only — Private pool LPs use withdraw_all / withdraw_base
// which handle value extraction differently.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct ClaimFees<'info> {
    #[account(
        mut,
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
        constraint = pool.is_active  @ PoolError::PoolNotActive,
        constraint = pool.pool_type == PoolType::Public @ PoolError::PoolTypeMismatch
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), asset.mint.as_ref()],
        bump = asset.bump,
    )]
    pub asset: Account<'info, AssetAccount>,

    /// Pool token vault for the asset being claimed.
    #[account(
        mut,
        constraint = pool_vault.owner == pool.key() @ PoolError::Unauthorized,
        constraint = pool_vault.mint  == asset.mint @ PoolError::AssetNotAllowed,
    )]
    pub pool_vault: Account<'info, TokenAccount>,

    /// User's token account — receives the fee tokens.
    #[account(mut)]
    pub user_token: Account<'info, TokenAccount>,

    /// Per-(pool, asset, user) deposit tracker.
    #[account(
        mut,
        seeds = [
            LP_DEPOSIT_SEED,
            pool.key().as_ref(),
            asset.mint.as_ref(),
            user.key().as_ref(),
        ],
        bump = lp_deposit.bump,
    )]
    pub lp_deposit: Account<'info, LpDepositAccount>,

    pub user: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

pub fn handler_claim_fees(ctx: Context<ClaimFees>) -> Result<()> {
    let pool       = &mut ctx.accounts.pool;
    let asset      = &mut ctx.accounts.asset;
    let lp_deposit = &mut ctx.accounts.lp_deposit;

    // ── COMPUTE CLAIMABLE FEES ────────────────────
    // claimable = pending_fees + amount × (fps − fee_debt) / FEE_SCALE
    let fps_delta    = asset.fees_per_share.saturating_sub(lp_deposit.fee_debt);
    let full_accrued = (lp_deposit.amount as u128)
        .checked_mul(fps_delta as u128)
        .unwrap_or(0)
        .checked_div(FEE_SCALE as u128)
        .unwrap_or(0) as u64;
    let claimable = lp_deposit.pending_fees
        .checked_add(full_accrued)
        .ok_or(PoolError::MathOverflow)?;

    require!(claimable > 0, PoolError::InsufficientBalance);
    require!(asset.amount >= claimable, PoolError::InsufficientLiquidity);

    // ── TRANSFER FEE TOKENS TO USER ───────────────
    let bump   = pool.bump;
    let seeds  = &[POOL_SEED, pool.owner.as_ref(), &[bump]];
    let signer = &[&seeds[..]];

    let cpi_accounts = Transfer {
        from:      ctx.accounts.pool_vault.to_account_info(),
        to:        ctx.accounts.user_token.to_account_info(),
        authority: pool.to_account_info(),
    };
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer,
        ),
        claimable,
    )?;

    // ── UPDATE STATE ──────────────────────────────
    // Only fee tokens leave the vault — principal (total_deposited) stays.
    asset.amount = asset.amount
        .checked_sub(claimable)
        .ok_or(PoolError::MathOverflow)?;

    // pool.total_value tracks deposited principal only — swap fees are NOT added
    // to total_value (they go to pool_weight in swap.rs STEP 9).  Do NOT touch
    // total_value here; subtracting fee tokens that were never added would cause
    // underflow on future withdrawals.
    pool.pool_weight = pool.pool_weight
        .checked_sub(claimable)
        .ok_or(PoolError::WeightError)?;

    // Reset fee tracking — future accruals start fresh from current fps.
    lp_deposit.pending_fees = 0;
    lp_deposit.fee_debt     = asset.fees_per_share;

    emit!(FeesClaimed {
        pool:      pool.key(),
        mint:      asset.mint,
        amount:    claimable,
        user:      ctx.accounts.user.key(),
    });

    Ok(())
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct FeesCompounded {
    pub pool:              Pubkey,
    pub mint:              Pubkey,
    /// Fee tokens reclassified as principal (no transfer — stayed in vault)
    pub compounded_amount: u64,
    /// LP's new total principal after compounding
    pub new_principal:     u64,
    pub user:              Pubkey,
}

#[event]
pub struct FeesClaimed {
    pub pool:   Pubkey,
    pub mint:   Pubkey,
    /// Total fee tokens transferred to the LP (pending_fees + newly accrued)
    pub amount: u64,
    pub user:   Pubkey,
}

#[event]
pub struct Deposited {
    pub pool:   Pubkey,
    pub mint:   Pubkey,
    pub amount: u64,
    pub user:   Pubkey,
}

#[event]
pub struct WithdrewBase {
    pub pool:   Pubkey,
    pub amount: u64,
    pub user:   Pubkey,
}

#[event]
pub struct WithdrewAll {
    pub pool:       Pubkey,
    pub mint:       Pubkey,
    pub amount:     u64,
    pub percentage: u8,
    pub user:       Pubkey,
}

#[event]
pub struct PublicExited {
    pub pool:      Pubkey,
    pub mint:      Pubkey,
    /// Principal tokens returned to the LP
    pub principal: u64,
    /// Fee share paid out alongside the principal
    pub fee_share: u64,
    /// Total tokens sent to user (principal + fee_share)
    pub total_out: u64,
    pub user:      Pubkey,
}
