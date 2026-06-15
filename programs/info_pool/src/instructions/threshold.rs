use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::InfoPoolError;
use crate::utils::*;

use pool_program::cpi as pool_cpi;
use pool_program::cpi::accounts::{UpdateFee, BlockInflow, UnblockInflow};

// ═══════════════════════════════════════════════════
// RUN THRESHOLD CHECK
// 3-Layer confirmation system
// Called after every Pyth feed update by the crank.
//
// On every call:
//   1. Evaluate 3-layer confirmation (TWAP, Volume, CI)
//   2. Determine new threshold state (Neutral / Approaching / Exceeded)
//   3. Recalculate dynamic fee
//   4. Decide whether to block/unblock inflow
//   5. CPI → Pool Program to push new fee and block/unblock state
//   6. If all 3 layers confirmed → shift price base (genuine growth)
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct RunThresholdCheck<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    /// Pool Program — verified against POOL_PROGRAM_ID constant
    /// CHECK: Program ID checked in handler before CPI
    pub pool_program: AccountInfo<'info>,

    /// Pool PDA in Pool program (mut for fee/block CPI)
    /// CHECK: Validated by Pool program's PDA constraints
    #[account(mut)]
    pub pool_account: AccountInfo<'info>,

    /// Asset PDA in Pool program (mut for fee/block CPI)
    /// CHECK: Validated by Pool program's PDA constraints
    #[account(mut)]
    pub asset_account: AccountInfo<'info>,

    /// Crank authority — must match info_pool.authority.
    /// Prevents any arbitrary signer from manipulating threshold state and fees.
    #[account(
        constraint = crank.key() == info_pool.authority @ InfoPoolError::NotCrank
    )]
    pub crank: Signer<'info>,
}

pub fn handler_check(
    ctx: Context<RunThresholdCheck>,
    mint: Pubkey,
) -> Result<()> {
    // ── SNAPSHOT SIGNER SEEDS DATA BEFORE MUTABLE BORROW ──
    // (Pubkey and u8 are Copy so this is safe)
    let pool_id_snap = ctx.accounts.info_pool.pool_id;
    let bump_snap    = ctx.accounts.info_pool.bump;

    // ── ALL COMPUTATION IN A SCOPED MUTABLE BORROW ────────
    // The scope ends before we make CPI calls so the borrow
    // is released and we can pass info_pool.to_account_info().
    let (new_fee, old_fee, should_block, was_blocked) = {
        let info_pool = &mut ctx.accounts.info_pool;
        let clock     = Clock::get()?;

        // ── FIND ASSET ────────────────────────────────
        let asset_idx = info_pool.assets
            .iter()
            .position(|a| a.mint == mint)
            .ok_or(InfoPoolError::AssetNotFound)?;

        let asset = &info_pool.assets[asset_idx];

        let current_price  = asset.pyth_data.price;
        let current_base   = asset.current_base;
        let threshold_up   = asset.threshold_up;
        let threshold_down = asset.threshold_down;
        let fee_min        = asset.fee_min;
        let fee_max        = asset.fee_max;
        let was_blocked    = asset.is_blocked;
        let old_fee        = asset.current_fee;

        // ── 3-LAYER CONFIRMATION ──────────────────────

        // Layer 1: TWAP — all 3 timeframes trending same direction?
        let twap_ok = check_twap_layer(
            asset.pyth_data.twap_short,
            asset.pyth_data.twap_medium,
            asset.pyth_data.twap_long,
            current_price,
        );

        // Layer 2: Volume — consistently rising (≥10% increase)?
        let volume_ok = check_volume_layer(
            asset.pyth_data.volume_24h,
            asset.pyth_data.volume_prev,
        );

        // Layer 3: Confidence — narrow CI (< CONFIDENCE_RATIO_BPS% of price)?
        let confidence_ok = check_confidence_layer(
            current_price,
            asset.pyth_data.confidence,
        );

        let all_confirmed = twap_ok && volume_ok && confidence_ok;

        // ── UPDATE LAYER STATUS ───────────────────────
        let asset_mut = &mut info_pool.assets[asset_idx];
        asset_mut.layer_status.twap_confirmed       = twap_ok;
        asset_mut.layer_status.volume_confirmed     = volume_ok;
        asset_mut.layer_status.confidence_confirmed = confidence_ok;
        asset_mut.layer_status.all_confirmed        = all_confirmed;
        if all_confirmed {
            asset_mut.layer_status.last_confirmed = clock.unix_timestamp;
        }

        // ── CALCULATE THRESHOLD STATE ─────────────────
        let new_state = calculate_threshold_state(
            current_price,
            current_base,
            threshold_up,
            threshold_down,
        );
        asset_mut.threshold_state = new_state.clone();

        // ── CALCULATE AND STORE NEW FEE ───────────────
        let new_fee = calculate_fee(
            current_price,
            current_base,
            threshold_up,
            threshold_down,
            fee_min,
            fee_max,
        );
        asset_mut.current_fee = new_fee;

        // ── SHOULD BLOCK INFLOW? ──────────────────────
        let should_block = should_block_inflow(&new_state);
        asset_mut.is_blocked = should_block;

        info_pool.last_updated = clock.unix_timestamp;

        // ── GENUINE GROWTH → SHIFT BASE ───────────────
        if all_confirmed {
            // saturating_sub prevents i64 overflow when current_price ≫ current_base
            // (e.g. first crank after a large price move before base has caught up).
            let confirmed_growth = current_price.saturating_sub(current_base);
            if confirmed_growth.abs() > 0 {
                let shift    = calculate_base_shift(current_base, confirmed_growth);
                let new_base = current_base
                    .checked_add(shift)
                    .ok_or(InfoPoolError::MathOverflow)?;
                require!(new_base > 0, InfoPoolError::ZeroBasePrice);
                info_pool.assets[asset_idx].current_base = new_base;

                emit!(ThresholdBaseShifted {
                    pool_id:  info_pool.pool_id,
                    mint,
                    old_base: current_base,
                    new_base,
                    shift,
                });
            }
        }

        emit!(ThresholdChecked {
            pool_id:      info_pool.pool_id,
            mint,
            twap_ok,
            volume_ok,
            confidence_ok,
            all_confirmed,
        });

        // Return values needed for CPI (after mutable borrow ends)
        (new_fee, old_fee, should_block, was_blocked)
    }; // ← mutable borrow of info_pool released here

    // ── VERIFY POOL PROGRAM IDENTITY ──────────────
    require!(
        ctx.accounts.pool_program.key().to_string() == POOL_PROGRAM_ID,
        InfoPoolError::NotPoolProgram
    );

    // ── BUILD SIGNER SEEDS (info_pool PDA) ────────
    let pool_id_bytes = pool_id_snap.to_bytes();
    let ip_seeds: &[&[u8]] = &[INFO_POOL_SEED, &pool_id_bytes, &[bump_snap]];
    let signer_seeds = &[ip_seeds];

    // ── CPI 1: PUSH FEE TO POOL PROGRAM ──────────
    // Only call if fee actually changed to save compute
    if new_fee != old_fee {
        pool_cpi::update_fee(
            CpiContext::new_with_signer(
                ctx.accounts.pool_program.to_account_info(),
                UpdateFee {
                    pool:                ctx.accounts.pool_account.to_account_info(),
                    asset:               ctx.accounts.asset_account.to_account_info(),
                    info_pool_authority: ctx.accounts.info_pool.to_account_info(),
                },
                signer_seeds,
            ),
            mint,
            new_fee,
        )?;
    }

    // ── CPI 2: BLOCK OR UNBLOCK INFLOW ────────────
    if should_block && !was_blocked {
        pool_cpi::block_inflow(
            CpiContext::new_with_signer(
                ctx.accounts.pool_program.to_account_info(),
                BlockInflow {
                    pool:                ctx.accounts.pool_account.to_account_info(),
                    asset:               ctx.accounts.asset_account.to_account_info(),
                    info_pool_authority: ctx.accounts.info_pool.to_account_info(),
                },
                signer_seeds,
            ),
            mint,
        )?;

        emit!(InflowBlockedByInfoPool {
            pool_id: pool_id_snap,
            mint,
        });

    } else if !should_block && was_blocked {
        pool_cpi::unblock_inflow(
            CpiContext::new_with_signer(
                ctx.accounts.pool_program.to_account_info(),
                UnblockInflow {
                    pool:                ctx.accounts.pool_account.to_account_info(),
                    asset:               ctx.accounts.asset_account.to_account_info(),
                    info_pool_authority: ctx.accounts.info_pool.to_account_info(),
                },
                signer_seeds,
            ),
            mint,
        )?;

        emit!(InflowUnblockedByInfoPool {
            pool_id: pool_id_snap,
            mint,
        });
    }

    Ok(())
}

// ═══════════════════════════════════════════════════
// UPDATE THRESHOLD BASE (manual governance override)
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct UpdateThresholdBase<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    pub governance_authority: Signer<'info>,
}

pub fn handler_update_base(
    ctx: Context<UpdateThresholdBase>,
    mint: Pubkey,
    confirmed_growth: i64,
) -> Result<()> {
    // Verify caller is the Governance program PDA (owned by governance program)
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
        .find(|a| a.mint == mint)
        .ok_or(InfoPoolError::AssetNotFound)?;

    let old_base = asset.current_base;
    let shift    = calculate_base_shift(old_base, confirmed_growth);
    let new_base = old_base
        .checked_add(shift)
        .ok_or(InfoPoolError::MathOverflow)?;

    require!(new_base > 0, InfoPoolError::ZeroBasePrice);

    asset.current_base = new_base;

    emit!(ThresholdBaseShifted {
        pool_id:  info_pool.pool_id,
        mint,
        old_base,
        new_base,
        shift,
    });

    Ok(())
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct ThresholdChecked {
    pub pool_id:       Pubkey,
    pub mint:          Pubkey,
    pub twap_ok:       bool,
    pub volume_ok:     bool,
    pub confidence_ok: bool,
    pub all_confirmed: bool,
}

#[event]
pub struct ThresholdBaseShifted {
    pub pool_id:  Pubkey,
    pub mint:     Pubkey,
    pub old_base: i64,
    pub new_base: i64,
    pub shift:    i64,
}

#[event]
pub struct InflowBlockedByInfoPool {
    pub pool_id: Pubkey,
    pub mint:    Pubkey,
}

#[event]
pub struct InflowUnblockedByInfoPool {
    pub pool_id: Pubkey,
    pub mint:    Pubkey,
}
