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

    /// Outgoing asset account (asset leaving pool — e.g. SOL)
    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), asset_out.mint.as_ref()],
        bump = asset_out.bump,
    )]
    pub asset_out: Account<'info, AssetAccount>,

    /// Incoming asset account (asset entering pool — e.g. USDC)
    #[account(
        mut,
        seeds = [ASSET_SEED, pool.key().as_ref(), asset_in.mint.as_ref()],
        bump = asset_in.bump,
    )]
    pub asset_in: Account<'info, AssetAccount>,

    /// Pool vault for outgoing asset (SOL).
    /// Must be owned by this pool PDA and hold the correct mint.
    /// TODO (Bug #5): add seeds constraint once vaults are created as PDAs:
    ///   seeds = [VAULT_SEED, pool.key().as_ref(), asset_out.mint.as_ref()]
    #[account(
        mut,
        constraint = pool_vault_out.owner == pool.key() @ PoolError::Unauthorized,
        constraint = pool_vault_out.mint  == asset_out.mint @ PoolError::AssetNotAllowed,
    )]
    pub pool_vault_out: Account<'info, TokenAccount>,

    /// Pool vault for incoming asset (USDC).
    /// Must be owned by this pool PDA and hold the correct mint.
    /// TODO (Bug #5): add seeds constraint once vaults are created as PDAs:
    ///   seeds = [VAULT_SEED, pool.key().as_ref(), asset_in.mint.as_ref()]
    #[account(
        mut,
        constraint = pool_vault_in.owner == pool.key() @ PoolError::Unauthorized,
        constraint = pool_vault_in.mint  == asset_in.mint @ PoolError::AssetNotAllowed,
    )]
    pub pool_vault_in: Account<'info, TokenAccount>,

    /// User's token account for outgoing asset (receives SOL)
    #[account(mut)]
    pub user_token_out: Account<'info, TokenAccount>,

    /// User's token account for incoming asset (sends USDC)
    #[account(mut)]
    pub user_token_in: Account<'info, TokenAccount>,

    /// User authority
    pub user: Signer<'info>,

    pub token_program: Program<'info, Token>,
}

/// Oracle-rate based swap with outgoing-only fee distribution.
///
/// `amount_in`      — tokens user is sending (e.g. USDC amount)
/// `min_amount_out` — minimum tokens user will accept (slippage guard)
///
/// ## Fee model (outgoing-only, pool-wide USD-normalised distribution)
///
/// Only the OUTGOING asset generates a fee:
///   fee_out   = gross_amount_out × asset_out.current_fee / BPS_DENOMINATOR
///   User receives amount_out = gross_amount_out − fee_out.
///   fee_out stays in the vault and is tracked in asset_out.fee_balance.
///
/// The outgoing fee is USD-normalised before crediting pool.pool_fps (Bug #2 fix):
///   fee_usd  = fee_out × rate_out / (ORACLE_PRICE_SCALE × 10^decimals)
///   pool_fps += fee_usd × FEE_SCALE / pool_total_lp_deposited_usd
///
/// ## Oracle staleness guard (Bug #3 fix)
///
/// Both asset_in.oracle_price_slot and asset_out.oracle_price_slot must be
/// within MAX_ORACLE_STALENESS_SLOTS of the current slot, or the swap is
/// rejected with OraclePriceStale.  This prevents execution at stale rates
/// when the InfoPool crank is down.
///
/// ## Max % concentration guard (hard reject)
///
/// Before executing, the handler computes the post-swap oracle-adjusted value of
/// asset_in relative to the two swap assets.  If asset_in's share would exceed
/// max_pct_max + MAX_PCT_BUFFER it rejects with MaxPctBufferExceeded.
pub fn handler(
    ctx: Context<Swap>,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<()> {
    let pool      = &mut ctx.accounts.pool;
    let asset_out = &mut ctx.accounts.asset_out;
    let asset_in  = &mut ctx.accounts.asset_in;

    // ── STEP 1: CHECK ASSET INTERACTION ──────────
    let interaction_allowed = asset_out.allowed.contains(&asset_in.mint);
    require!(interaction_allowed, PoolError::InteractionNotAllowed);

    // ── STEP 2: CHECK INFLOW BLOCKED ─────────────
    require!(!asset_in.is_blocked, PoolError::InflowBlocked);

    // ── STEP 3: (concentration guard delegated to InfoPool threshold engine)

    // ── STEP 4: READ ORACLE RATES + STALENESS CHECK (Bug #3) ──
    let rate_in  = asset_in.oracle_price;
    let rate_out = asset_out.oracle_price;
    require!(rate_in  > 0, PoolError::OraclePriceNotSet);
    require!(rate_out > 0, PoolError::OraclePriceNotSet);

    // Reject if oracle price has not been refreshed recently.
    // oracle_price_slot is set by the update_oracle_price CPI; a zero value
    // means InfoPool has never pushed a price for this asset.
    let current_slot = Clock::get()?.slot;
    require!(
        asset_in.oracle_price_slot > 0
            && current_slot.saturating_sub(asset_in.oracle_price_slot)
                <= MAX_ORACLE_STALENESS_SLOTS,
        PoolError::OraclePriceStale
    );
    require!(
        asset_out.oracle_price_slot > 0
            && current_slot.saturating_sub(asset_out.oracle_price_slot)
                <= MAX_ORACLE_STALENESS_SLOTS,
        PoolError::OraclePriceStale
    );

    require!(asset_out.amount > 0, PoolError::InsufficientLiquidity);

    // ── STEP 5: CALCULATE AMOUNT OUT (ORACLE RATE) ──
    let amount_out_before_fee = (amount_in as u128)
        .checked_mul(rate_in as u128)
        .ok_or(PoolError::MathOverflow)?
        .checked_div(rate_out as u128)
        .ok_or(PoolError::MathOverflow)? as u64;

    // ── STEP 5.5: MAX % CONCENTRATION GUARD ──────
    {
        let new_in_amount = asset_in.amount
            .checked_add(amount_in)
            .ok_or(PoolError::MathOverflow)?;

        let new_in_usd = (new_in_amount as u128)
            .checked_mul(rate_in as u128)
            .ok_or(PoolError::MathOverflow)?;

        let post_out_amount = asset_out.amount
            .saturating_sub(amount_out_before_fee);

        let post_out_usd = (post_out_amount as u128)
            .checked_mul(rate_out as u128)
            .ok_or(PoolError::MathOverflow)?;

        let total_two_usd = new_in_usd
            .checked_add(post_out_usd)
            .ok_or(PoolError::MathOverflow)?;

        if total_two_usd > 0 {
            let in_pct_bps = new_in_usd
                .checked_mul(10_000)
                .ok_or(PoolError::MathOverflow)?
                .checked_div(total_two_usd)
                .ok_or(PoolError::MathOverflow)? as u64;

            let hard_cap_bps =
                (asset_in.max_pct_max as u64)
                    .checked_add(MAX_PCT_BUFFER as u64)
                    .ok_or(PoolError::MathOverflow)?
                    .checked_mul(100)
                    .ok_or(PoolError::MathOverflow)?;

            require!(in_pct_bps <= hard_cap_bps, PoolError::MaxPctBufferExceeded);
        }
    }

    // ── STEP 6: APPLY OUTGOING FEE ───────────────
    let out_fee_bps    = asset_out.current_fee as u64;
    let out_fee_amount = amount_out_before_fee
        .checked_mul(out_fee_bps)
        .ok_or(PoolError::MathOverflow)?
        .checked_div(BPS_DENOMINATOR)
        .ok_or(PoolError::MathOverflow)?;

    let amount_out = amount_out_before_fee
        .checked_sub(out_fee_amount)
        .ok_or(PoolError::MathOverflow)?;

    require!(
        asset_out.amount >= amount_out,
        PoolError::InsufficientLiquidity
    );

    // ── STEP 7: SLIPPAGE CHECK ────────────────────
    require!(amount_out >= min_amount_out, PoolError::SlippageExceeded);

    // ── STEP 8: EXECUTE TOKEN TRANSFERS ──────────

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

    let pool_key  = pool.key();
    let bump      = pool.bump;
    let seeds     = &[POOL_SEED, pool.owner.as_ref(), &[bump]];
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

    asset_in.amount = asset_in.amount
        .checked_add(amount_in)
        .ok_or(PoolError::MathOverflow)?;

    asset_out.amount = asset_out.amount
        .checked_sub(amount_out)
        .ok_or(PoolError::MathOverflow)?;

    // Bug #1 fix: track per-asset fee balance instead of relying on pool_weight
    // (which mixed different token units and caused WeightError fund locks).
    // fee_balance is decremented by claim_fees / public_exit when LP harvests fees.
    asset_out.fee_balance = asset_out.fee_balance
        .checked_add(out_fee_amount)
        .ok_or(PoolError::MathOverflow)?;

    // pool_weight is kept as an informational routing tiebreaker only.
    // It is NOT used for fee-claim gating after Bug #1 fix.
    // saturating_add: mixed-unit overflow is benign here (routing uses it as a
    // relative rank, not an absolute value).
    pool.pool_weight = pool.pool_weight.saturating_add(out_fee_amount);

    // ── STEP 10: UPDATE POOL-WIDE FEE ACCUMULATOR (Bug #2 fix) ─
    //
    // Fee is USD-normalised before dividing by pool_total_lp_deposited_usd so
    // that the fps accumulator is unit-consistent across all asset types.
    //
    // fee_usd = out_fee_amount × rate_out
    //           / (ORACLE_PRICE_SCALE × 10^asset_out.decimals)
    //
    // This converts a fee expressed in native token units (lamports, micro-USDC…)
    // into a USD-equivalent value that is comparable across all pool assets.
    if pool.pool_total_lp_deposited_usd > 0 && out_fee_amount > 0 {
        let token_scale = 10u128.pow(asset_out.decimals as u32);
        let fee_usd = (out_fee_amount as u128)
            .checked_mul(rate_out as u128)
            .unwrap_or(0)
            .checked_div(ORACLE_PRICE_SCALE as u128)
            .unwrap_or(0)
            .checked_div(token_scale)
            .unwrap_or(0);

        if fee_usd > 0 {
            let fps_inc = fee_usd
                .checked_mul(FEE_SCALE as u128)
                .unwrap_or(0)
                .checked_div(pool.pool_total_lp_deposited_usd as u128)
                .unwrap_or(0) as u64;
            pool.pool_fps = pool.pool_fps.saturating_add(fps_inc);
        }
    }

    emit!(SwapExecuted {
        pool:           pool_key,
        asset_in:       asset_in.mint,
        asset_out:      asset_out.mint,
        amount_in,
        amount_out,
        out_fee_amount,
        out_fee_bps:    out_fee_bps as u16,
        rate_in,
        rate_out,
    });

    Ok(())
}

#[event]
pub struct SwapExecuted {
    pub pool:           Pubkey,
    pub asset_in:       Pubkey,
    pub asset_out:      Pubkey,
    pub amount_in:      u64,
    pub amount_out:     u64,
    /// Fee kept in pool vault on the outgoing asset, distributed to all LPs
    pub out_fee_amount: u64,
    /// Fee rate used for outgoing asset (basis points)
    pub out_fee_bps:    u16,
    /// Oracle price of asset_in — sourced from InfoPool, not user-supplied
    pub rate_in:        u64,
    /// Oracle price of asset_out — sourced from InfoPool, not user-supplied
    pub rate_out:       u64,
}
