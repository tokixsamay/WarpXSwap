use anchor_lang::prelude::*;

pub mod instructions;
pub mod state;
pub mod errors;
pub mod constants;
pub mod utils;

#[cfg(test)]
mod tests;

use instructions::*;
use state::{PoolType, AddAssetParams};

declare_id!("4AXtXF5VWeWKLqP6vHKPpjoc7wQ8r4duDqZ4CENtzsqZ");

#[program]
pub mod pool_program {
    use super::*;

    // ── POOL SETUP ──────────────────────────────

    pub fn initialize_pool(
        ctx: Context<InitializePool>,
        pool_type: PoolType,
    ) -> Result<()> {
        instructions::initialize_pool::handler(ctx, pool_type)
    }

    pub fn add_asset(
        ctx: Context<AddAsset>,
        params: AddAssetParams,
    ) -> Result<()> {
        instructions::add_asset::handler(ctx, params)
    }

    pub fn remove_asset(
        ctx: Context<RemoveAsset>,
    ) -> Result<()> {
        instructions::allowance::handler_remove_asset(ctx)
    }

    pub fn set_allowance(
        ctx: Context<SetAllowance>,
        target_mint: Pubkey,
        allowed: bool,
    ) -> Result<()> {
        instructions::allowance::handler_set_allowance(ctx, target_mint, allowed)
    }

    // ── LP INSTRUCTIONS ──────────────────────────

    pub fn deposit(
        ctx: Context<Deposit>,
        amount: u64,
    ) -> Result<()> {
        instructions::deposit_withdraw::handler_deposit(ctx, amount)
    }

    pub fn withdraw_base(
        ctx: Context<WithdrawBase>,
        amount: u64,
    ) -> Result<()> {
        instructions::deposit_withdraw::handler_withdraw_base(ctx, amount)
    }

    pub fn withdraw_all(
        ctx: Context<WithdrawAll>,
        percentage: u8,
    ) -> Result<()> {
        instructions::deposit_withdraw::handler_withdraw_all(ctx, percentage)
    }

    pub fn public_exit(
        ctx: Context<PublicExit>,
        amount: u64,
    ) -> Result<()> {
        instructions::deposit_withdraw::handler_public_exit(ctx, amount)
    }

    /// Withdraw accrued swap-fee yield to the LP's token account.
    /// Principal is unchanged. fee_debt is reset to current accumulator.
    /// Public pools only.
    pub fn claim_fees(ctx: Context<ClaimFees>) -> Result<()> {
        instructions::deposit_withdraw::handler_claim_fees(ctx)
    }

    /// Reclassify accrued fee yield as principal — no token transfer.
    /// Fees are already in the vault; this is a pure accounting update.
    /// Public pools only.
    pub fn compound_fees(ctx: Context<CompoundFees>) -> Result<()> {
        instructions::deposit_withdraw::handler_compound_fees(ctx)
    }

    // ── READ-ONLY QUERIES ─────────────────────────

    /// Returns the caller's current withdrawable LP balance for one asset.
    ///
    /// No signer authority required — anyone can query any depositor's balance.
    /// The result is encoded as Anchor return data (u64) AND emitted as an
    /// `LpDepositBalance` event for indexers.
    ///
    /// Alternatively, derive the PDA off-chain and call
    /// `program.account.lpDepositAccount.fetch(pda)` — no transaction needed.
    pub fn get_lp_deposit_balance(
        ctx: Context<GetLpDepositBalance>,
    ) -> Result<u64> {
        instructions::read::handler_get_lp_deposit_balance(ctx)
    }

    /// Returns the total fee share this LP can claim on their next public_exit.
    ///
    /// Computed as: pending_fees + principal × (fees_per_share − fee_debt) / FEE_SCALE
    /// On a partial exit, the LP receives a pro-rated fraction of this value.
    ///
    /// No signer required — read-only simulation call.
    /// An `LpClaimableFees` event is also emitted for indexers.
    pub fn get_lp_claimable_fees(
        ctx: Context<GetLpClaimableFees>,
    ) -> Result<u64> {
        instructions::read::handler_get_lp_claimable_fees(ctx)
    }

    // ── SWAP ──────────────────────────────────────

    /// Oracle-rate based swap.
    /// Rates are read from AssetAccount.oracle_price, pushed by InfoPool
    /// via update_oracle_price CPI. No user-supplied rates accepted.
    pub fn swap(
        ctx: Context<Swap>,
        amount_in: u64,
        min_amount_out: u64,
    ) -> Result<()> {
        instructions::swap::handler(ctx, amount_in, min_amount_out)
    }

    // ── CALLED BY INFO POOL (CPI) ─────────────────

    /// Push current Pyth spot price into AssetAccount.oracle_price.
    /// Called by InfoPool crank after each update_pyth_feeds tick.
    /// This avoids a circular CPI dep (Pool → InfoPool → Pool) by
    /// having InfoPool push prices into Pool's own state instead.
    pub fn update_oracle_price(
        ctx: Context<UpdateOraclePrice>,
        mint: Pubkey,
        price: u64,
    ) -> Result<()> {
        instructions::info_pool_cpi::handler_update_oracle_price(ctx, mint, price)
    }

    pub fn update_fee(
        ctx: Context<UpdateFee>,
        mint: Pubkey,
        new_fee: u16,
    ) -> Result<()> {
        instructions::info_pool_cpi::handler_update_fee(ctx, mint, new_fee)
    }

    pub fn block_inflow(
        ctx: Context<BlockInflow>,
        mint: Pubkey,
    ) -> Result<()> {
        instructions::info_pool_cpi::handler_block_inflow(ctx, mint)
    }

    pub fn unblock_inflow(
        ctx: Context<UnblockInflow>,
        mint: Pubkey,
    ) -> Result<()> {
        instructions::info_pool_cpi::handler_unblock_inflow(ctx, mint)
    }

    // ── CALLED BY GOVERNANCE (CPI) ────────────────

    pub fn governance_update_fee_range(
        ctx: Context<GovernanceUpdateFeeRange>,
        mint: Pubkey,
        new_min: u16,
        new_max: u16,
    ) -> Result<()> {
        instructions::governance_cpi::handler_update_fee_range(ctx, mint, new_min, new_max)
    }

    pub fn governance_update_threshold(
        ctx: Context<GovernanceUpdateThreshold>,
        mint: Pubkey,
        new_up: u16,
        new_down: u16,
    ) -> Result<()> {
        instructions::governance_cpi::handler_update_threshold(ctx, mint, new_up, new_down)
    }

    pub fn governance_update_max_pct(
        ctx: Context<GovernanceUpdateMaxPct>,
        mint: Pubkey,
        new_min: u8,
        new_max: u8,
    ) -> Result<()> {
        instructions::governance_cpi::handler_update_max_pct(ctx, mint, new_min, new_max)
    }

    pub fn governance_add_asset(
        ctx: Context<GovernanceAddAsset>,
        params: AddAssetParams,
    ) -> Result<()> {
        instructions::governance_cpi::handler_governance_add_asset(ctx, params)
    }

    pub fn governance_remove_asset(
        ctx: Context<GovernanceRemoveAsset>,
    ) -> Result<()> {
        instructions::governance_cpi::handler_governance_remove_asset(ctx)
    }

    pub fn governance_set_allowance(
        ctx: Context<GovernanceSetAllowance>,
        asset_mint: Pubkey,
        target_mint: Pubkey,
        allowed: bool,
    ) -> Result<()> {
        instructions::governance_cpi::handler_governance_set_allowance(
            ctx, asset_mint, target_mint, allowed,
        )
    }

    /// Governance manually blocks or unblocks inflow for an asset.
    /// `blocked = true`  → circuit-breaker (Pool stays private).
    /// `blocked = false` → re-opens the asset; threshold_state reset to Neutral.
    pub fn governance_set_inflow_blocked(
        ctx: Context<GovernanceSetInflowBlocked>,
        mint: Pubkey,
        blocked: bool,
    ) -> Result<()> {
        instructions::governance_cpi::handler_governance_set_inflow_blocked(ctx, mint, blocked)
    }
}
