use anchor_lang::prelude::*;
use crate::state::{AssetAccount, LpDepositAccount, PoolAccount};
use crate::constants::{LP_DEPOSIT_SEED, ASSET_SEED, POOL_SEED, FEE_SCALE};
use crate::errors::PoolError;

// ═══════════════════════════════════════════════════
// GET LP DEPOSIT BALANCE
//
// Returns the current net deposit amount for a given
// (pool, asset_mint, depositor) triple.
//
// Who can call it: anyone — no signer authority needed.
// This is a read-only query; it modifies no state.
//
// Return value:
//   The instruction returns `u64` as Anchor return data
//   (available on-chain via CPI and off-chain via
//   `connection.simulateTransaction` or tx returnData).
//   An `LpDepositBalance` event is also emitted so
//   indexers and listeners can pick up the value
//   without a simulation round-trip.
//
// Frontend derivation (TypeScript, no transaction needed):
//   const [pda] = PublicKey.findProgramAddressSync(
//     [
//       Buffer.from("lp_deposit"),
//       poolKey.toBuffer(),
//       assetMint.toBuffer(),
//       depositorKey.toBuffer(),
//     ],
//     POOL_PROGRAM_ID,
//   );
//   const account = await program.account.lpDepositAccount.fetch(pda);
//   // account.amount — u64 — current withdrawable principal
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct GetLpDepositBalance<'info> {
    /// Per-(pool, asset, depositor) deposit tracker.
    /// Seeds are verified from the stored fields on the PDA itself
    /// so the caller does not need to pass extra pool/mint accounts.
    #[account(
        seeds = [
            LP_DEPOSIT_SEED,
            lp_deposit.pool.as_ref(),
            lp_deposit.asset.as_ref(),
            lp_deposit.depositor.as_ref(),
        ],
        bump = lp_deposit.bump,
    )]
    pub lp_deposit: Account<'info, LpDepositAccount>,
}

pub fn handler_get_lp_deposit_balance(
    ctx: Context<GetLpDepositBalance>,
) -> Result<u64> {
    let lp = &ctx.accounts.lp_deposit;

    emit!(LpDepositBalance {
        pool:      lp.pool,
        asset:     lp.asset,
        depositor: lp.depositor,
        amount:    lp.amount,
    });

    Ok(lp.amount)
}

// ─── EVENT ────────────────────────────────────────
#[event]
pub struct LpDepositBalance {
    pub pool:      Pubkey,
    pub asset:     Pubkey,
    pub depositor: Pubkey,
    /// Net cumulative principal: deposits minus public-exits.
    /// Does NOT include unclaimed fee share.
    pub amount:    u64,
}

// ═══════════════════════════════════════════════════
// GET LP CLAIMABLE FEES
//
// Computes the fee share an LP can claim on their next public_exit
// without sending any transaction.  Simulate or fetch the PDA directly.
//
// Formula (matches public_exit / claim_fees logic exactly):
//   fps_delta      = pool.pool_fps − lp_deposit.fee_debt
//   full_accrued   = lp_deposit.amount × fps_delta / FEE_SCALE
//   claimable_fees = lp_deposit.pending_fees + full_accrued
//
// Note: this is the TOTAL claimable for the entire position.
// On a partial exit the LP receives a pro-rated fraction.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct GetLpClaimableFees<'info> {
    /// Pool account — needed for pool.pool_fps (the pool-wide fee accumulator).
    #[account(
        seeds = [POOL_SEED, pool.owner.as_ref()],
        bump = pool.bump,
        constraint = pool.key() == lp_deposit.pool @ PoolError::Unauthorized,
    )]
    pub pool: Account<'info, PoolAccount>,

    #[account(
        seeds = [
            LP_DEPOSIT_SEED,
            lp_deposit.pool.as_ref(),
            lp_deposit.asset.as_ref(),
            lp_deposit.depositor.as_ref(),
        ],
        bump = lp_deposit.bump,
    )]
    pub lp_deposit: Account<'info, LpDepositAccount>,

    /// Asset account for this pool+mint pair — read to verify PDA derivation.
    #[account(
        seeds = [
            ASSET_SEED,
            lp_deposit.pool.as_ref(),
            lp_deposit.asset.as_ref(),
        ],
        bump = asset.bump,
    )]
    pub asset: Account<'info, AssetAccount>,
}

pub fn handler_get_lp_claimable_fees(
    ctx: Context<GetLpClaimableFees>,
) -> Result<u64> {
    let pool = &ctx.accounts.pool;
    let lp   = &ctx.accounts.lp_deposit;

    // Pool-wide model: fee_debt and pool_fps both track the same accumulator.
    // asset.fees_per_share is deprecated and is always 0.
    let fps_delta    = pool.pool_fps.saturating_sub(lp.fee_debt);
    let full_accrued = (lp.amount as u128)
        .checked_mul(fps_delta as u128)
        .unwrap_or(0)
        .checked_div(FEE_SCALE as u128)
        .unwrap_or(0) as u64;
    let claimable = lp.pending_fees.saturating_add(full_accrued);

    emit!(LpClaimableFees {
        pool:           lp.pool,
        asset:          lp.asset,
        depositor:      lp.depositor,
        principal:      lp.amount,
        claimable_fees: claimable,
        pool_fps:       pool.pool_fps,
        fee_debt:       lp.fee_debt,
    });

    Ok(claimable)
}

#[event]
pub struct LpClaimableFees {
    pub pool:           Pubkey,
    pub asset:          Pubkey,
    pub depositor:      Pubkey,
    /// Current principal (does not include fees)
    pub principal:      u64,
    /// Total fee tokens claimable on next public_exit or claim_fees
    pub claimable_fees: u64,
    /// Current pool-wide fee accumulator (pool.pool_fps)
    pub pool_fps:       u64,
    /// LP's debt snapshot (pool_fps value at last deposit or settle)
    pub fee_debt:       u64,
}
