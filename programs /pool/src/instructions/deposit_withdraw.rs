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
//
// Bug #2 fix: at deposit time, the native-token amount is USD-normalised
// using the current oracle price and stored in lp_deposit.amount_usd and
// pool.pool_total_lp_deposited_usd.  These USD fields are the authoritative
// denominators for pool_fps fee distribution, so LPs with equal USD value
// across assets (e.g. 1 SOL ≈ 150 USDC) receive equal fee shares regardless
// of each token's decimal precision (SOL=9, USDC=6).
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

    // Bug #23 fix: enforce a minimum initial deposit for private pools.
    //
    // Private pools are owner-controlled vaults — there is no public LP
    // mechanism to dilute a dust position.  Allowing a dust first deposit
    // would create a PoolAccount with governance weight but no real liquidity,
    // and leave pool_fps denominator pathologically small.
    //
    // The check fires ONLY when total_value == 0 (first-ever deposit) for a
    // private pool.  Subsequent deposits have no floor.
    //
    // MIN_PRIVATE_POOL_INITIAL_DEPOSIT = 1_000_000_000 base units (1 wSOL / 1 token).
    if pool.pool_type == PoolType::Private && pool.total_value == 0 {
        require!(
            amount >= MIN_PRIVATE_POOL_INITIAL_DEPOSIT,
            PoolError::InsufficientInitialValue,
        );
    }

    // NOTE: LP deposits are NOT subject to the inflow block.
    // Only external swaps (swap.rs) check is_blocked.
    // This allows LPs to always add liquidity even during threshold events,
    // which is intentional — the LP is the pool operator, not an external trader.

    // ── GOVERNANCE REGISTRATION (Bug #17 note) ────
    // This deposit instruction does NOT call the Governance program via CPI.
    // Coupling Pool deposits to Governance CPIs would require passing extra
    // accounts on every deposit, add latency, and create a hard dependency on
    // the governance program being deployed and initialised.
    //
    // Governance participation is a MANUAL, SEPARATE step:
    //   1. First deposit (this instruction) — grants LP rights in the pool.
    //   2. Call `register_contributor` (governance program) with the same wallet
    //      to create a ContributorAccount and enter the top-10 leaderboard.
    //   3. On subsequent deposits/withdrawals, call `update_contributor_stake`
    //      so the governance leaderboard stays in sync with actual LP holdings.
    //
    // The SDK exposes helpers for steps 2 and 3:
    //   sdk.governance.registerContributor(poolId, wallet, stakeAmount)
    //   sdk.governance.updateContributorStake(poolId, wallet, newStake)
    //
    // Setup scripts MUST call these after every deposit to avoid voting-power
    // drift. LPs whose deposits are not registered will hold pool positions but
    // have zero governance influence until they register.

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

    // ── USD NORMALISATION (Bug #2 fix) ────────────
    // amount_usd = amount × oracle_price / (ORACLE_PRICE_SCALE × 10^decimals)
    //
    // oracle_price encodes USD per whole token × ORACLE_PRICE_SCALE.
    //   e.g. SOL at $150 → oracle_price = 150_000_000 (ORACLE_PRICE_SCALE=1e6)
    //
    // If oracle_price == 0 (pre-first-price-push), fall back to using the
    // native amount directly.  This prevents a zero denominator in pool_fps
    // while still allowing LPs to seed liquidity before crank starts.
    let amount_usd: u64 = if asset.oracle_price > 0 {
        let token_scale = 10u128.pow(asset.decimals as u32);
        (amount as u128)
            .checked_mul(asset.oracle_price as u128)
            .unwrap_or(0)
            .checked_div(ORACLE_PRICE_SCALE as u128)
            .unwrap_or(0)
            .checked_div(token_scale)
            .unwrap_or(0) as u64
    } else {
        amount
    };

    // ── TRACK LP CONTRIBUTION ─────────────────────
    // Pool-wide fee model (USD-normalised):
    //   fee_debt tracks pool.pool_fps checkpoint.
    //   amount_usd is the USD share used as the numerator in fee claims.
    //   pending_fees stores settled USD-valued fees on re-deposit.
    //
    // First deposit: init PDA, set fee_debt = current pool_fps so the LP does
    //   not retroactively claim fees earned before they joined.
    // Re-deposit: settle accrued USD-valued fees into pending_fees BEFORE
    //   resetting fee_debt, preserving all earnings.
    let is_first_deposit = lp_deposit.pool == Pubkey::default();
    if is_first_deposit {
        lp_deposit.pool         = pool.key();
        lp_deposit.asset        = asset.mint;
        lp_deposit.depositor    = ctx.accounts.user.key();
        lp_deposit.bump         = ctx.bumps.lp_deposit;
        lp_deposit.fee_debt     = pool.pool_fps;
        lp_deposit.pending_fees = 0;
        lp_deposit.amount_usd   = 0;
    } else {
        // Settle accrued USD-valued fees into pending_fees before updating fee_debt.
        // Uses lp_deposit.amount_usd (Bug #2) so the share is unit-correct.
        let fps_delta = pool.pool_fps.saturating_sub(lp_deposit.fee_debt);
        let accrued_usd = (lp_deposit.amount_usd as u128)
            .checked_mul(fps_delta as u128)
            .unwrap_or(0)
            .checked_div(FEE_SCALE as u128)
            .unwrap_or(0) as u64;
        lp_deposit.pending_fees = lp_deposit.pending_fees
            .checked_add(accrued_usd)
            .ok_or(PoolError::MathOverflow)?;
        lp_deposit.fee_debt = pool.pool_fps;
    }

    lp_deposit.amount = lp_deposit.amount
        .checked_add(amount)
        .ok_or(PoolError::MathOverflow)?;
    lp_deposit.amount_usd = lp_deposit.amount_usd
        .checked_add(amount_usd)
        .ok_or(PoolError::MathOverflow)?;

    // ── TRACK TOTAL PRINCIPAL PER ASSET ──────────
    asset.total_deposited = asset.total_deposited
        .checked_add(amount)
        .ok_or(PoolError::MathOverflow)?;

    // ── TRACK POOL-WIDE LP DEPOSITS ───────────────
    // pool_total_lp_deposited_usd is the fps denominator (Bug #2 fix).
    // pool_total_lp_deposited is kept for reference only.
    // Both are incremented only for public pools — private pools use
    // withdraw_base / withdraw_all which do not touch these fields.
    if pool.pool_type == PoolType::Public {
        pool.pool_total_lp_deposited = pool.pool_total_lp_deposited
            .checked_add(amount)
            .ok_or(PoolError::MathOverflow)?;
        pool.pool_total_lp_deposited_usd = pool.pool_total_lp_deposited_usd
            .checked_add(amount_usd)
            .ok_or(PoolError::MathOverflow)?;
    }

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
    #[account(
        mut,
        constraint = pool_vault_base.owner == pool.key() @ PoolError::Unauthorized,
        constraint = pool_vault_base.mint  == base_asset.mint @ PoolError::AssetNotAllowed,
    )]
    pub pool_vault_base: Account<'info, TokenAccount>,

    #[account(mut)]
    pub user_token_base: Account<'info, TokenAccount>,

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

    let bump  = pool.bump;
    let seeds = &[POOL_SEED, pool.owner.as_ref(), &[bump]];
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
//
// Bug #1 fix: fee_share is gated by asset.fee_balance (per-asset fee vault)
//   rather than pool.pool_weight (mixed-unit, pool-wide — caused WeightError
//   underflow and permanent fund lock-up).
//
// Bug #2 fix: fee computation uses USD-normalised amount_usd for the share
//   numerator; claimable_native is converted back via current oracle price.
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

    #[account(
        mut,
        constraint = pool_vault.owner == pool.key() @ PoolError::Unauthorized,
        constraint = pool_vault.mint  == asset.mint @ PoolError::AssetNotAllowed,
    )]
    pub pool_vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub user_token: Account<'info, TokenAccount>,

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
    require!(lp_deposit.amount >= amount, PoolError::ExceedsDeposit);

    // ── COMPUTE FEE SHARE (USD-normalised, Bug #2 fix) ──
    //
    // 1. fps_delta × lp_deposit.amount_usd / FEE_SCALE = accrued_usd
    //    (amount_usd is USD-normalised principal, matching pool_fps denominator)
    // 2. Pro-rate by exit fraction (amount / lp_deposit.amount) for partial exits.
    // 3. Convert USD claimable → native tokens via current oracle price.
    // 4. Gate by asset.fee_balance (Bug #1 fix).
    let fps_delta = pool.pool_fps.saturating_sub(lp_deposit.fee_debt);

    let full_accrued_usd = (lp_deposit.amount_usd as u128)
        .checked_mul(fps_delta as u128)
        .unwrap_or(0)
        .checked_div(FEE_SCALE as u128)
        .unwrap_or(0) as u64;

    // Pro-rate by exit fraction
    let exit_fee_accrued_usd = (full_accrued_usd as u128)
        .checked_mul(amount as u128)
        .unwrap_or(0)
        .checked_div(lp_deposit.amount as u128)
        .unwrap_or(0) as u64;

    let exit_pending_usd = (lp_deposit.pending_fees as u128)
        .checked_mul(amount as u128)
        .unwrap_or(0)
        .checked_div(lp_deposit.amount as u128)
        .unwrap_or(0) as u64;

    let fee_share_usd = exit_fee_accrued_usd
        .checked_add(exit_pending_usd)
        .ok_or(PoolError::MathOverflow)?;

    // Convert USD fee share → native tokens at current oracle price.
    // Requires oracle price to be set (should be true post-first-swap).
    // If oracle_price == 0 (pre-crank), skip fee payment (safe — no fees earned yet).
    let fee_share: u64 = if fee_share_usd > 0 && asset.oracle_price > 0 {
        let token_scale = 10u128.pow(asset.decimals as u32);
        (fee_share_usd as u128)
            .checked_mul(ORACLE_PRICE_SCALE as u128)
            .unwrap_or(0)
            .checked_mul(token_scale)
            .unwrap_or(0)
            .checked_div(asset.oracle_price as u128)
            .unwrap_or(0) as u64
    } else {
        0
    };

    // ── LIQUIDITY CHECK (Bug #1 fix) ──────────────
    // Guard principal withdrawal against vault balance.
    let total_out = amount
        .checked_add(fee_share)
        .ok_or(PoolError::MathOverflow)?;
    require!(asset.amount >= total_out, PoolError::InsufficientBalance);

    // Guard fee payout against per-asset fee_balance (Bug #1 fix).
    // Replaces old pool.pool_weight.checked_sub → WeightError which crossed
    // asset unit boundaries and caused spurious fund locks.
    if fee_share > 0 {
        require!(
            asset.fee_balance >= fee_share,
            PoolError::InsufficientFeeBalance
        );
    }

    // ── TRANSFER principal + fee_share ────────────
    let bump   = pool.bump;
    let seeds  = &[POOL_SEED, pool.owner.as_ref(), &[bump]];
    let signer = &[&seeds[..]];

    let cpi_accounts = Transfer {
        from:      ctx.accounts.pool_v
