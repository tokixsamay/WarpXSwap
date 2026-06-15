use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;
use crate::state::*;
use crate::constants::*;
use crate::errors::RoutingError;

use info_pool_program::state::{InfoPoolAccount, ThresholdState as IpThreshold};

// ═══════════════════════════════════════════════════
// FIND BEST POOL
// Iterates candidate InfoPool PDAs via remaining_accounts,
// applies filter + priority algorithm, returns best match.
//
// Account ordering in remaining_accounts (per candidate):
//   [InfoPoolAccount PDA]   — must match candidate_pools[i]
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct FindBestPool<'info> {
    #[account(
        seeds = [ROUTER_SEED],
        bump = router_config.bump,
        constraint = router_config.is_active @ RoutingError::RouterNotActive
    )]
    pub router_config: Account<'info, RouterConfig>,

    /// Caller (Pool Program or user)
    pub caller: Signer<'info>,
}

pub fn handler_find_best(
    ctx: Context<FindBestPool>,
    params: FindBestPoolParams,
) -> Result<RouteResult> {

    require!(params.amount_in > 0,              RoutingError::InvalidAmount);
    require!(!params.candidate_pools.is_empty(), RoutingError::NoCandidates);
    require!(
        params.candidate_pools.len() <= MAX_CANDIDATES,
        RoutingError::TooManyCandidates
    );
    // rate_in / rate_out are read from each candidate's InfoPool below

    // Caller must supply one InfoPool PDA per candidate in remaining_accounts
    require!(
        ctx.remaining_accounts.len() == params.candidate_pools.len(),
        RoutingError::NoCandidates
    );

    let mut candidates: Vec<PoolCandidate> = Vec::new();

    for (i, pool_key) in params.candidate_pools.iter().enumerate() {
        let info_pool_ai = &ctx.remaining_accounts[i];

        // Verify the InfoPool account is owned by the registered Info Pool program
        let expected_program: Pubkey = ctx.accounts.router_config.info_pool_program;
        if *info_pool_ai.owner != expected_program {
            continue;
        }

        // Deserialise InfoPool account data
        let ip_data = info_pool_ai.try_borrow_data()?;
        let ip: InfoPoolAccount = InfoPoolAccount::try_deserialize(&mut &ip_data[..])?;

        // Confirm InfoPool belongs to the expected pool
        if ip.pool_id != *pool_key {
            continue;
        }

        // Find asset_in and asset_out entries
        let ip_asset_out = match ip.assets.iter().find(|a| a.mint == params.asset_out) {
            Some(a) => a,
            None    => continue,
        };
        let ip_asset_in = match ip.assets.iter().find(|a| a.mint == params.asset_in) {
            Some(a) => a,
            None    => continue,
        };

        // Oracle rates sourced from InfoPool — not from user-supplied params.
        // Guard: Pyth price is i64; a negative price (stale/error) cast to u64 wraps
        // to an astronomically large number and produces catastrophic swap output.
        // Skip this candidate rather than propagating bad data.
        let raw_price_in  = ip_asset_in.pyth_data.price;
        let raw_price_out = ip_asset_out.pyth_data.price;
        if raw_price_in <= 0 || raw_price_out <= 0 {
            continue;
        }
        let rate_in  = raw_price_in  as u64;
        let rate_out = raw_price_out as u64;

        // Read real values from InfoPool
        let asset_out_allowed_list  = ip_asset_out.allowed.clone();
        let asset_in_is_blocked     = ip_asset_in.is_blocked;
        let asset_out_is_blocked    = ip_asset_out.is_blocked;
        let current_fee             = ip_asset_out.current_fee;
        let pool_weight             = ip.pool_weight;
        let pool_liquidity          = ip.pool_size;
        let pool_is_active          = pool_liquidity > 0;
        let volume_confirmed        = ip_asset_out.layer_status.volume_confirmed;
        let all_confirmed           = ip_asset_out.layer_status.all_confirmed;

        let threshold_pct = threshold_to_pct(&ip_asset_out.threshold_state);

        let passes = filter_hard_rules(
            &asset_out_allowed_list,
            &params.asset_in,
            asset_in_is_blocked,
            asset_out_is_blocked,
            pool_liquidity,
            params.amount_in,
            current_fee,
            params.max_fee_bps,
            pool_is_active,
        );

        if passes {
            let priority = assign_priority(threshold_pct);
            candidates.push(PoolCandidate {
                pool:             *pool_key,
                asset_out_fee:    current_fee,
                pool_weight,
                priority,
                liquidity:        pool_liquidity,
                is_blocked:       asset_in_is_blocked,
                all_confirmed,
                volume_confirmed,
                rate_in,
                rate_out,
            });
        }
    }

    require!(!candidates.is_empty(), RoutingError::NoPoolFound);

    let best = select_best(&mut candidates)
        .ok_or(RoutingError::NoPoolFound)?
        .clone();

    // NOTE: best.rate_in / best.rate_out were sourced from InfoPool above.
    // We cannot re-read them here without another deserialization, so we skip
    // the amount_out estimate in find_best (it is only a preview; execute.rs
    // does the authoritative calculation with InfoPool-sourced rates).
    let (amount_out, _) = calc_amount_out_oracle(
        params.amount_in,
        best.rate_in,
        best.rate_out,
        best.asset_out_fee,
    ).ok_or(RoutingError::InsufficientLiquidity)?;

    let priority_u8 = match best.priority {
        Priority::P1Exceeded    => 1u8,
        Priority::P2Approaching => 2u8,
        Priority::P3Neutral     => 3u8,
    };

    emit!(RouteFound {
        asset_in:  params.asset_in,
        asset_out: params.asset_out,
        amount_in: params.amount_in,
    });

    Ok(RouteResult {
        best_pool:        best.pool,
        expected_out:     amount_out,
        fee_bps:          best.asset_out_fee,
        priority:         priority_u8,
        pool_weight:      best.pool_weight,
        volume_confirmed: best.volume_confirmed,
        all_confirmed:    best.all_confirmed,
    })
}

// ═══════════════════════════════════════════════════
// GET QUOTE
// Reads real fee from InfoPool PDA and computes oracle-rate quote.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct GetQuote<'info> {
    #[account(
        seeds = [ROUTER_SEED],
        bump = router_config.bump,
    )]
    pub router_config: Account<'info, RouterConfig>,

    pub caller: Signer<'info>,

    /// InfoPool PDA for the target pool — provides live fee data
    /// CHECK: owner verified in handler; no custom constraint needed
    pub info_pool: AccountInfo<'info>,

    /// Pool vault for asset_out — liquidity availability check
    pub pool_vault_out: Account<'info, TokenAccount>,
}

pub fn handler_get_quote(
    ctx: Context<GetQuote>,
    params: QuoteParams,
) -> Result<QuoteResult> {

    require!(params.amount_in > 0, RoutingError::InvalidAmount);

    // Verify InfoPool is owned by the registered Info Pool program
    require!(
        *ctx.accounts.info_pool.owner == ctx.accounts.router_config.info_pool_program,
        RoutingError::NoPoolFound
    );

    // Deserialise InfoPool account
    let ip_data = ctx.accounts.info_pool.try_borrow_data()?;
    let ip: InfoPoolAccount = InfoPoolAccount::try_deserialize(&mut &ip_data[..])?;

    // Confirm InfoPool belongs to the expected pool
    require!(ip.pool_id == params.pool, RoutingError::NoPoolFound);

    // Find asset_in and asset_out entries — oracle rates sourced from InfoPool
    let ip_asset_in = ip.assets.iter()
        .find(|a| a.mint == params.asset_in)
        .ok_or(RoutingError::NoPoolFound)?;
    let ip_asset_out = ip.assets.iter()
        .find(|a| a.mint == params.asset_out)
        .ok_or(RoutingError::NoPoolFound)?;

    // Read oracle rates from InfoPool state — not from user-supplied params.
    // Check positivity on the i64 field BEFORE casting: a negative Pyth price
    // cast to u64 wraps to a huge number and `> 0` would pass incorrectly.
    require!(ip_asset_in.pyth_data.price  > 0, RoutingError::InvalidAmount);
    require!(ip_asset_out.pyth_data.price > 0, RoutingError::InvalidAmount);
    let rate_in  = ip_asset_in.pyth_data.price  as u64;
    let rate_out = ip_asset_out.pyth_data.price as u64;

    let fee_bps          = ip_asset_out.current_fee;
    let volume_confirmed = ip_asset_out.layer_status.volume_confirmed;
    let all_confirmed    = ip_asset_out.layer_status.all_confirmed;

    // Oracle-rate based output calculation using InfoPool prices
    let (amount_out, fee_amount) = calc_amount_out_oracle(
        params.amount_in,
        rate_in,
        rate_out,
        fee_bps,
    ).ok_or(RoutingError::InsufficientLiquidity)?;

    // Confirm pool vault has enough balance to pay out
    require!(
        ctx.accounts.pool_vault_out.amount >= amount_out,
        RoutingError::InsufficientLiquidity
    );

    emit!(QuoteCalculated {
        pool:       params.pool,
        asset_in:   params.asset_in,
        asset_out:  params.asset_out,
        amount_in:  params.amount_in,
        amount_out,
        fee_bps,
        volume_confirmed,
        all_confirmed,
    });

    Ok(QuoteResult {
        pool: params.pool,
        amount_out,
        fee_amount,
        fee_bps,
        volume_confirmed,
        all_confirmed,
    })
}

// ═══════════════════════════════════════════════════
// FILTER + PRIORITY CORE LOGIC
// Pure functions — no account reads needed
// ═══════════════════════════════════════════════════

/// Convert InfoPool ThresholdState to a signed percentage (for priority)
pub fn threshold_to_pct(state: &IpThreshold) -> i8 {
    match state {
        IpThreshold::Neutral              => 0,
        IpThreshold::ApproachingUp(p)     => *p as i8,
        IpThreshold::ApproachingDown(p)   => -(*p as i8),
        IpThreshold::ExceededUp           => 100,
        IpThreshold::ExceededDown         => -100,
    }
}

/// Apply Filter 1: Hard rules. Returns true if pool passes all checks.
pub fn filter_hard_rules(
    asset_out_allowed_assets: &[Pubkey],
    asset_in_mint:            &Pubkey,
    asset_in_is_blocked:      bool,
    asset_out_is_blocked:     bool,
    pool_liquidity:           u64,
    amount_in:                u64,
    current_fee:              u16,
    max_fee_bps:              u16,
    pool_is_active:           bool,
) -> bool {
    if !asset_out_allowed_assets.contains(asset_in_mint) { return false; }
    // Block if the incoming asset OR outgoing asset has hit a threshold breach.
    // asset_in  blocked → stop flooding a concentrated asset in.
    // asset_out blocked → stop draining an asset that is itself under stress.
    if asset_in_is_blocked                               { return false; }
    if asset_out_is_blocked                              { return false; }
    if pool_liquidity < amount_in                        { return false; }
    if pool_liquidity < MIN_LIQUIDITY                    { return false; }
    if current_fee > max_fee_bps                         { return false; }
    if !pool_is_active                                   { return false; }
    true
}

/// Assign priority: P1 if threshold exceeded, P2 if approaching, P3 neutral
pub fn assign_priority(threshold_state_pct: i8) -> Priority {
    let abs_pct = threshold_state_pct.unsigned_abs();
    if abs_pct >= 100 {
        Priority::P1Exceeded
    } else if abs_pct >= P2_THRESHOLD_PCT {
        Priority::P2Approaching
    } else {
        Priority::P3Neutral
    }
}

/// Select best candidate: lowest fee within highest priority, tiebreak by
/// all_confirmed (confirmed beats unconfirmed), then pool weight.
pub fn select_best(candidates: &mut Vec<PoolCandidate>) -> Option<&PoolCandidate> {
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by(|a, b| {
        // 1. Highest priority tier first (P1 < P2 < P3 in enum order)
        let p = a.priority.cmp(&b.priority);
        if p != std::cmp::Ordering::Equal { return p; }
        // 2. Lowest fee
        let f = a.asset_out_fee.cmp(&b.asset_out_fee);
        if f != std::cmp::Ordering::Equal { return f; }
        // 3. all_confirmed preferred
        let ac = b.all_confirmed.cmp(&a.all_confirmed);
        if ac != std::cmp::Ordering::Equal { return ac; }
        // 4. Highest pool weight as final tiebreaker
        b.pool_weight.cmp(&a.pool_weight)
    });
    candidates.first()
}

/// Oracle-rate based pricing:
///   amount_out = (amount_in × rate_in) / rate_out
///   fee applied to amount_out (outgoing asset only)
///
/// Returns (amount_out_after_fee, fee_amount) or None on overflow/zero.
pub fn calc_amount_out_oracle(
    amount_in: u64,
    rate_in:   u64,
    rate_out:  u64,
    fee_bps:   u16,
) -> Option<(u64, u64)> {
    if rate_in == 0 || rate_out == 0 {
        return None;
    }

    let raw_out = (amount_in as u128)
        .checked_mul(rate_in as u128)?
        .checked_div(rate_out as u128)? as u64;

    let fee_amount = raw_out
        .checked_mul(fee_bps as u64)?
        .checked_div(BPS_DENOMINATOR)?;

    let amount_out = raw_out.checked_sub(fee_amount)?;
    Some((amount_out, fee_amount))
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct RouteFound {
    pub asset_in:   Pubkey,
    pub asset_out:  Pubkey,
    pub amount_in:  u64,
}

#[event]
pub struct QuoteCalculated {
    pub pool:             Pubkey,
    pub asset_in:         Pubkey,
    pub asset_out:        Pubkey,
    pub amount_in:        u64,
    pub amount_out:       u64,
    pub fee_bps:          u16,
    pub volume_confirmed: bool,
    pub all_confirmed:    bool,
}
