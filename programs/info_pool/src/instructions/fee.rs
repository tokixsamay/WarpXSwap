use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::InfoPoolError;
use crate::utils::*;

use pool_program::cpi as pool_cpi;
use pool_program::cpi::accounts::UpdateFee;

// ═══════════════════════════════════════════════════
// CALCULATE AND PUSH FEE
// Recalculate fee in InfoPool and CPI to Pool Program to sync.
// Called by the crank as a lightweight fee update (no 3-layer check).
// For the full 3-layer check + fee push, use run_threshold_check instead.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct CalculateAndPushFee<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    /// Pool Program — verified against POOL_PROGRAM_ID constant
    /// CHECK: Program ID checked in handler before CPI
    pub pool_program: AccountInfo<'info>,

    /// Pool PDA in Pool program (mut for fee CPI)
    /// CHECK: Validated by Pool program's PDA constraints
    #[account(mut)]
    pub pool_account: AccountInfo<'info>,

    /// Asset PDA in Pool program (mut for fee CPI)
    /// CHECK: Validated by Pool program's PDA constraints
    #[account(mut)]
    pub asset_account: AccountInfo<'info>,

    pub crank: Signer<'info>,
}

pub fn handler(
    ctx: Context<CalculateAndPushFee>,
    mint: Pubkey,
) -> Result<()> {
    // ── VERIFY POOL PROGRAM IDENTITY ──────────────
    // pool_program is an AccountInfo (not a signer); check its key == program ID.
    require!(
        ctx.accounts.pool_program.key().to_string() == POOL_PROGRAM_ID,
        InfoPoolError::NotPoolProgram
    );

    // ── SNAPSHOT PDA SEEDS BEFORE MUTABLE BORROW ──
    let pool_id_snap = ctx.accounts.info_pool.pool_id;
    let bump_snap    = ctx.accounts.info_pool.bump;

    // ── COMPUTE NEW FEE IN SCOPED BORROW ──────────
    let (new_fee, old_fee, is_stable) = {
        let info_pool = &mut ctx.accounts.info_pool;

        let asset = info_pool.assets
            .iter_mut()
            .find(|a| a.mint == mint)
            .ok_or(InfoPoolError::AssetNotFound)?;

        let old_fee   = asset.current_fee;
        let is_stable = asset.is_stable;

        let new_fee = if is_stable {
            // ── STABLECOIN: use LP-set static fee, skip V-shape curve ──
            asset.static_fee_bps
        } else {
            // ── VOLATILE ASSET: V-shape fee curve ──
            let fee = calculate_fee(
                asset.pyth_data.price,
                asset.current_base,
                asset.threshold_up,
                asset.threshold_down,
                asset.fee_min,
                asset.fee_max,
            );
            // Clamp within LP-set bounds
            fee.max(asset.fee_min).min(asset.fee_max)
        };

        asset.current_fee = new_fee;

        (new_fee, old_fee, is_stable)
    }; // ← mutable borrow of info_pool released here

    // ── CPI: PUSH FEE TO POOL PROGRAM ─────────────
    // Only call if fee actually changed to save compute units.
    if new_fee != old_fee {
        let pool_id_bytes = pool_id_snap.to_bytes();
        let ip_seeds: &[&[u8]] = &[INFO_POOL_SEED, &pool_id_bytes, &[bump_snap]];
        let signer_seeds = &[ip_seeds];

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

    emit!(FeePushed {
        pool_id: pool_id_snap,
        mint,
        old_fee,
        new_fee,
        is_stable,
    });

    Ok(())
}

#[event]
pub struct FeePushed {
    pub pool_id:  Pubkey,
    pub mint:     Pubkey,
    pub old_fee:  u16,
    pub new_fee:  u16,
    pub is_stable: bool,
}
