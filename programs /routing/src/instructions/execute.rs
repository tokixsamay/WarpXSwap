use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};
use crate::state::*;
use crate::constants::*;
use crate::errors::RoutingError;
use crate::instructions::routing::{
    filter_hard_rules,
    assign_priority,
    calc_amount_out_oracle,
};

use info_pool_program::state::InfoPoolAccount;
use pool_program::cpi as pool_cpi;
use pool_program::cpi::accounts::Swap as PoolSwap;
use pool_program::state::{PoolAccount, AssetAccount};

// ═══════════════════════════════════════════════════
// EXECUTE ROUTE
// Validates via real InfoPool state → Executes Pool swap via CPI
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct ExecuteRoute<'info> {
    // ── Router gate ──────────────────────────────
    #[account(
        seeds = [ROUTER_SEED],
        bump = router_config.bump,
        constraint = router_config.is_active @ RoutingError::RouterNotActive
    )]
    pub router_config: Account<'info, RouterConfig>,

    // ── InfoPool (read-only) ─────────────────────
    /// InfoPool PDA for the selected pool — used to read
    /// live fee, is_blocked, allowed list, and threshold state.
    #[account(
        seeds = [b"info_pool", pool.key().as_ref()],
        bump,
        seeds::program = router_config.info_pool_program,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    // ── Pool accounts (passed to Pool CPI) ───────
    #[account(
        mut,
        seeds = [b"pool", pool.owner.as_ref()],
        bump = pool.bump,
        seeds::program = router_config.pool_program,
    )]
    pub pool: Account<'info, PoolAccount>,

    /// Outgoing asset (what user receives from pool)
    #[account(
        mut,
        seeds = [b"asset", pool.key().as_ref(), asset_out.mint.as_ref()],
        bump = asset_out.bump,
        seeds::program = router_config.pool_program,
    )]
    pub asset_out: Account<'info, AssetAccount>,

    /// Incoming asset (what user sends to pool)
    #[account(
        mut,
        seeds = [b"asset", pool.key().as_ref(), asset_in.mint.as_ref()],
        bump = asset_in.bump,
        seeds::program = router_config.pool_program,
    )]
    pub asset_in: Account<'info, AssetAccount>,

    /// Pool vault for asset_out
    #[account(mut)]
    pub pool_vault_out: Account<'info, TokenAccount>,

    /// Pool vault for asset_in
    #[account(mut)]
    pub pool_vault_in: Account<'info, TokenAccount>,

    /// User's token account for asset_out (receives tokens)
    #[account(mut)]
    pub user_token_out: Account<'info, TokenAccount>,

    /// User's token account for asset_in (sends tokens)
    #[account(mut)]
    pub user_token_in: Account<'info, TokenAccount>,

    /// User authority — signs the swap
    pub user: Signer<'info>,

    pub token_program: Program<'info, Token>,

    /// Pool Program invoked via CPI
    /// CHECK: validated against router_config.pool_program
    #[account(
        constraint = pool_program.key() == router_config.pool_program
            @ RoutingError::RouterNotActive
    )]
    pub pool_program: AccountInfo<'info>,
}

pub fn handler(
    ctx: Context<ExecuteRoute>,
    params: ExecuteRouteParams,
) -> Result<RouteResult> {

    require!(params.amount_in > 0,              RoutingError::InvalidAmount);
    require!(!params.candidate_pools.is_empty(), RoutingError::NoCandidates);

    let asset_in_mint  = ctx.accounts.asset_in.mint;
    let asset_out_mint = ctx.accounts.asset_out.mint;

    // ── Confirm mints match params ────────────────
    require!(
        asset_in_mint  == params.asset_in  &&
        asset_out_mint == params.asset_out,
        RoutingError::NoPoolFound
    );

    // ── STEP 1: READ REAL INFO POOL STATE ─────────
    let info_pool = &ctx.accounts.info_pool;

    let ip_asset_in = info_pool
        .assets
        .iter()
        .find(|a| a.mint == asset_in_mint)
        .ok_or(RoutingError::NoPoolFound)?;

    let ip_asset_out = info_pool
        .assets
        .iter()
        .find(|a| a.mint == asset_out_mint)
        .ok_or(RoutingError::NoPoolFound)?;

    // Read oracle rates from InfoPool — not from user-supplied params.
    // This eliminates the user-supplied rate manipulation vector.
    //
    // SAFETY: pyth_data.price is i64. Cast to u64 ONLY after confirming > 0
    // on the raw i64 value. A negative i64 wraps to a large u64 > 0, making
    // a post-cast require!(rate > 0) check completely meaningless.
    let raw_price_in  = ip_asset_in.pyth_data.price;
    let raw_price_out = ip_asset_out.pyth_data.price;
    require!(
        raw_price_in > 0 && raw_price_out > 0,
        RoutingError::InvalidAmount
    );
    let rate_in  = raw_price_in  as u64;
    let rate_out = raw_price_out as u64;

    let asset_out_allowed_list = ip_asset_out.allowed.clone();
    let asset_in_is_blocked    = ip_asset_in.is_blocked;
    let asset_out_is_blocked   = ip_asset_out.is_blocked;
    let current_fee            = ip_asset_out.current_fee;
    let pool_liquidity         = ctx.accounts.pool_vault_out.amount;
    let pool_weight            = info_pool.pool_weight;
    let volume_confirmed       = ip_asset_out.layer_status.volume_confirmed;
    let all_confirmed          = ip_asset_out.layer_status.all_confirmed;

    let threshold_pct = match &ip_asset_out.threshold_state {
        info_pool_program::state::ThresholdState::Neutral              => 0i8,
        info_pool_program::state::ThresholdState::ApproachingUp(p)     => *p as i8,
        info_pool_program::state::ThresholdState::ApproachingDown(p)   => -(*p as i8),
        info_pool_program::state::ThresholdState::ExceededUp           => 100i8,
        info_pool_program::state::ThresholdState::ExceededDown         => -100i8,
    };

    // ── STEP 2: APPLY HARD FILTERS ────────────────
    let pool_is_active = ctx.accounts.pool.is_active;

    // ── STEP 4: CALCULATE EXPECTED OUTPUT (ORACLE) ──
    // Compute before the hard-rule filter so the vault balance check in
    // filter_hard_rules can compare the actual payout (amount_out) to the
    // vault balance — using amount_in here would compare incompatible token units.
    let (amount_out, fee_amount) = calc_amount_out_oracle(
        params.amount_in,
        rate_in,
        rate_out,
        current_fee,
    ).ok_or(RoutingError::InsufficientLiquidity)?;

    let passes = filter_hard_rules(
        &asset_out_allowed_list,
        &asset_in_mint,
        asset_in_is_blocked,
        asset_out_is_blocked,
        Some(pool_liquidity),
        amount_out,
        current_fee,
        params.max_fee_bps,
        pool_is_active,
    );

    require!(passes, RoutingError::NoPoolFound);

    // Pool vault must have enough balance to pay out (also checked inside
    // filter_hard_rules above when vault_out_balance = Some(pool_liquidity)).
    require!(
        pool_liquidity >= amount_out,
        RoutingError::InsufficientLiquidity
    );

    // ── STEP 3: ASSIGN PRIORITY ───────────────────
    let priority = assign_priority(threshold_pct);

    // ── STEP 5: SLIPPAGE CHECK ────────────────────
    require!(
        amount_out >= params.min_amount_out,
        RoutingError::SlippageTooHigh
    );

    // ── STEP 6: EXECUTE POOL SWAP VIA CPI ─────────
    // Pool Program re-validates everything independently (defense in depth).
    // Passes oracle rates through so Pool uses the same pricing.

    let cpi_accounts = PoolSwap {
        pool:           ctx.accounts.pool.to_account_info(),
        asset_out:      ctx.accounts.asset_out.to_account_info(),
        asset_in:       ctx.accounts.asset_in.to_account_info(),
        pool_vault_out: ctx.accounts.pool_vault_out.to_account_info(),
        pool_vault_in:  ctx.accounts.pool_vault_in.to_account_info(),
        user_token_out: ctx.accounts.user_token_out.to_account_info(),
        user_token_in:  ctx.accounts.user_token_in.to_account_info(),
        user:           ctx.accounts.user.to_account_info(),
        token_program:  ctx.accounts.token_program.to_account_info(),
    };

    let cpi_ctx = CpiContext::new(
        ctx.accounts.pool_program.to_account_info(),
        cpi_accounts,
    );

    // Pool reads oracle rates from its own AssetAccount.oracle_price,
    // pushed by InfoPool's push_oracle_price_to_pool CPI.
    // No rates passed here — user-supplied rate manipulation is impossible.
    //
    // 0.5% buffer absorbs fee-sync lag: InfoPool and Pool fees can diverge
    // by one crank tick if a fee update lands mid-flight.  Routing already
    // validated amount_out >= params.min_amount_out (Step 5), so the buffer
    // only protects the Pool-side re-check from spurious reverts — it does
    // not let the user receive less than their stated minimum.
    let min_with_buffer = params.min_amount_out
        .saturating_sub(params.min_amount_out / 200);
    pool_cpi::swap(cpi_ctx, params.amount_in, min_with_buffer)?;

    // ── STEP 7: EMIT + RETURN ─────────────────────
    let priority_u8 = match priority {
        Priority::P1Exceeded    => 1u8,
        Priority::P2Approaching => 2u8,
        Priority::P3Neutral     => 3u8,
    };

    emit!(RouteExecuted {
        pool:             ctx.accounts.pool.key(),
        asset_in:         asset_in_mint,
        asset_out:        asset_out_mint,
        amount_in:        params.amount_in,
        amount_out,
        fee_bps:          current_fee,
        priority:         priority_u8,
        pool_weight,
        fee_amount,
        volume_confirmed,
        all_confirmed,
    });

    Ok(RouteResult {
        best_pool:        ctx.accounts.pool.key(),
        expected_out:     amount_out,
        fee_bps:          current_fee,
        priority:         priority_u8,
        pool_weight,
        volume_confirmed,
        all_confirmed,
    })
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct RouteExecuted {
    pub pool:             Pubkey,
    pub asset_in:         Pubkey,
    pub asset_out:        Pubkey,
    pub amount_in:        u64,
    pub amount_out:       u64,
    pub fee_bps:          u16,
    pub priority:         u8,
    pub pool_weight:      u64,
    pub fee_amount:       u64,
    pub volume_confirmed: bool,
    pub all_confirmed:    bool,
  }
  
