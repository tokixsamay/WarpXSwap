use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::GovernanceError;

use pool_program::cpi as pool_cpi;
use pool_program::cpi::accounts::{
    GovernanceUpdateFeeRange,
    GovernanceUpdateThreshold as PoolGovernanceUpdateThreshold,
    GovernanceUpdateMaxPct,
    GovernanceAddAsset,
    GovernanceRemoveAsset,
    GovernanceSetAllowance,
    GovernanceSetInflowBlocked,
};
use pool_program::state::AddAssetParams;

use info_pool_program::cpi as info_pool_cpi;
use info_pool_program::cpi::accounts::{
    GovernanceUpdateThreshold as InfoGovernanceUpdateThreshold,
    GovernanceUpdateFeeRange  as InfoGovernanceUpdateFeeRange,
    GovernanceUpdateMaxPct    as InfoGovernanceUpdateMaxPct,
    GovernanceSetAllowance    as InfoGovernanceSetAllowance,
    GovernanceAddAsset        as InfoGovernanceAddAsset,
    GovernanceRemoveAsset     as InfoGovernanceRemoveAsset,
    GovernanceSetPythFeedId   as InfoGovernanceSetPythFeedId,
};

// ═══════════════════════════════════════════════════
// EXECUTE PROPOSAL
// Anyone can trigger execution after a proposal passes.
// The Governance PDA signs all CPIs via invoke_signed.
//
// Governance PDA seeds: [b"governance", pool_id]
// Both Pool and Info Pool programs verify this signer.
//
// AddAsset: executor pays rent for the new AssetAccount.
// RemoveAsset: rent is returned to the executor.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct ExecuteProposal<'info> {
    #[account(
        seeds = [GOVERNANCE_SEED, governance.pool_id.as_ref()],
        bump = governance.bump,
    )]
    pub governance: Account<'info, GovernanceAccount>,

    #[account(
        mut,
        seeds = [
            PROPOSAL_SEED,
            governance.pool_id.as_ref(),
            &proposal_id.to_le_bytes()
        ],
        bump = proposal.bump,
    )]
    pub proposal: Account<'info, ProposalAccount>,

    /// Pool Program — validated against the known program ID constant
    /// CHECK: Program ID checked in handler
    pub pool_program: AccountInfo<'info>,

    /// Info Pool Program — validated against INFO_POOL_PROGRAM_ID before any CPI
    /// CHECK: Program ID checked in handler against INFO_POOL_PROGRAM_ID constant
    pub info_pool_program: AccountInfo<'info>,

    /// Pool PDA (Pool program validates its seeds internally)
    /// CHECK: Validated inside Pool program
    #[account(mut)]
    pub pool_account: AccountInfo<'info>,

    /// Asset PDA (Pool program validates its seeds internally)
    /// CHECK: Validated inside Pool program
    #[account(mut)]
    pub asset_account: AccountInfo<'info>,

    /// Info Pool PDA (Info Pool program validates its seeds internally)
    /// CHECK: Validated inside Info Pool program
    #[account(mut)]
    pub info_pool_account: AccountInfo<'info>,

    /// Executor signs as payer for AddAsset (rent for new AssetAccount)
    /// and receives returned rent for RemoveAsset.
    #[account(mut)]
    pub executor: Signer<'info>,

    /// Required for AddAsset (init of AssetAccount PDA)
    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<ExecuteProposal>,
    _proposal_id: u64,
) -> Result<()> {
    // Snapshot immutable data before taking the mutable proposal borrow.
    let pool_id      = ctx.accounts.governance.pool_id;
    let gov_bump     = ctx.accounts.governance.bump;

    // Validate both cross-program accounts before any CPI.
    require!(
        ctx.accounts.pool_program.key().to_string() == POOL_PROGRAM_ID,
        GovernanceError::NotPoolProgram
    );
    require!(
        ctx.accounts.info_pool_program.key().to_string() == INFO_POOL_PROGRAM_ID,
        GovernanceError::NotInfoPoolProgram
    );

    let proposal = &mut ctx.accounts.proposal;

    require!(
        proposal.status == ProposalStatus::Passed,
        GovernanceError::ProposalNotPassed
    );
    require!(
        !proposal.executed,
        GovernanceError::AlreadyExecuted
    );

    // ── TIMELOCK CHECK ────────────────────────────
    // execute_after = 0 (devnet) means no delay.
    // On mainnet EXECUTE_DELAY_SECS should be ≥ 24 h so LPs can react.
    let now = Clock::get()?.unix_timestamp;
    require!(
        proposal.execute_after == 0 || now >= proposal.execute_after,
        GovernanceError::TimelockActive
    );

    // ── PDA SIGNER SEEDS ──────────────────────────
    // The governance PDA signs all downstream CPIs.
    let pool_id_ref  = pool_id.as_ref();
    let gov_seeds: &[&[u8]] = &[GOVERNANCE_SEED, pool_id_ref, &[gov_bump]];
    let signer_seeds = &[gov_seeds];

    // Snapshot the proposal_id for events (avoids re-borrow after mutation).
    let proposal_id_snap = proposal.proposal_id;

    // ── DISPATCH ──────────────────────────────────
    match proposal.payload.clone() {

        // ── UpdateFeeRange ────────────────────────
        // CPI 1 → Pool:      governance_update_fee_range
        //   Updates fee_min/fee_max on the AssetAccount PDA and clamps
        //   the stored current_fee within the new bounds.
        // CPI 2 → Info Pool: governance_update_fee_range
        //   Updates fee_min/fee_max on the AssetInfo so the next
        //   calculate_and_push_fee crank call uses the new curve.
        ProposalPayload::UpdateFeeRange { mint, new_min, new_max } => {
            pool_cpi::governance_update_fee_range(
                CpiContext::new_with_signer(
                    ctx.accounts.pool_program.to_account_info(),
                    GovernanceUpdateFeeRange {
                        pool:                 ctx.accounts.pool_account.to_account_info(),
                        asset:                ctx.accounts.asset_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                    },
                    signer_seeds,
                ),
                mint, new_min, new_max,
            )?;

            info_pool_cpi::governance_update_fee_range(
                CpiContext::new_with_signer(
                    ctx.accounts.info_pool_program.to_account_info(),
                    InfoGovernanceUpdateFeeRange {
                        info_pool:            ctx.accounts.info_pool_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                    },
                    signer_seeds,
                ),
                mint, new_min, new_max,
            )?;

            emit!(ProposalExecuted {
                pool_id,
                proposal_id:   proposal_id_snap,
                proposal_type: ProposalType::UpdateFeeRange,
                description:   format!(
                    "Fee range updated (Pool + Info Pool): mint={} {}–{}bps",
                    mint, new_min, new_max
                ),
            });
        }

        // ── UpdateThreshold ───────────────────────
        // CPI → Pool:      governance_update_threshold
        // CPI → Info Pool: governance_update_threshold
        // Both programs must share the same threshold bounds so the
        // 3-layer Pyth engine and the Pool fee gates stay in sync.
        ProposalPayload::UpdateThreshold { mint, new_up, new_down } => {
            pool_cpi::governance_update_threshold(
                CpiContext::new_with_signer(
                    ctx.accounts.pool_program.to_account_info(),
                    PoolGovernanceUpdateThreshold {
                        pool:                 ctx.accounts.pool_account.to_account_info(),
                        asset:                ctx.accounts.asset_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                    },
                    signer_seeds,
                ),
                mint, new_up, new_down,
            )?;

            info_pool_cpi::governance_update_threshold(
                CpiContext::new_with_signer(
                    ctx.accounts.info_pool_program.to_account_info(),
                    InfoGovernanceUpdateThreshold {
                        info_pool:            ctx.accounts.info_pool_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                    },
                    signer_seeds,
                ),
                mint, new_up, new_down,
            )?;

            emit!(ProposalExecuted {
                pool_id,
                proposal_id:   proposal_id_snap,
                proposal_type: ProposalType::UpdateThreshold,
                description:   format!(
                    "Thresholds updated (Pool + Info Pool): mint={} up={}bps down={}bps",
                    mint, new_up, new_down
                ),
            });
        }

        // ── UpdateMaxPct ──────────────────────────
        // CPI 1 → Pool:      governance_update_max_pct
        //   Updates max_pct_min/max_pct_max on the AssetAccount PDA.
        // CPI 2 → Info Pool: governance_update_max_pct
        //   Mirrors the new bounds onto AssetInfo so the Routing program
        //   can read current concentration limits from InfoPoolAccount
        //   without issuing a separate cross-program read.
        ProposalPayload::UpdateMaxPct { mint, new_min, new_max } => {
            pool_cpi::governance_update_max_pct(
                CpiContext::new_with_signer(
                    ctx.accounts.pool_program.to_account_info(),
                    GovernanceUpdateMaxPct {
                        pool:                 ctx.accounts.pool_account.to_account_info(),
                        asset:                ctx.accounts.asset_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                    },
                    signer_seeds,
                ),
                mint, new_min, new_max,
            )?;

            info_pool_cpi::governance_update_max_pct(
                CpiContext::new_with_signer(
                    ctx.accounts.info_pool_program.to_account_info(),
                    InfoGovernanceUpdateMaxPct {
                        info_pool:            ctx.accounts.info_pool_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                    },
                    signer_seeds,
                ),
                mint, new_min, new_max,
            )?;

            emit!(ProposalExecuted {
                pool_id,
                proposal_id:   proposal_id_snap,
                proposal_type: ProposalType::UpdateMaxPct,
                description:   format!(
                    "Max% updated (Pool + Info Pool): mint={} {}%–{}%",
                    mint, new_min, new_max
                ),
            });
        }

        // ── AddAsset ──────────────────────────────
        // CPI 1 → Pool:      governance_add_asset  (inits AssetAccount PDA)
        // CPI 2 → Info Pool: governance_add_asset  (registers asset in 3-layer engine)
        // Both must succeed or the transaction reverts atomically.
        // The executor pays rent for the new Pool AssetAccount PDA.
        ProposalPayload::AddAsset {
            mint, max_pct_min, max_pct_max,
            fee_min, fee_max, threshold_up,
            threshold_down, initial_base, allowed,
            is_stable, static_fee_bps,
        } => {
            pool_cpi::governance_add_asset(
                CpiContext::new_with_signer(
                    ctx.accounts.pool_program.to_account_info(),
                    GovernanceAddAsset {
                        pool:                 ctx.accounts.pool_account.to_account_info(),
                        asset:                ctx.accounts.asset_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                        payer:                ctx.accounts.executor.to_account_info(),
                        system_program:       ctx.accounts.system_program.to_account_info(),
                    },
                    signer_seeds,
                ),
                AddAssetParams {
                    mint,
                    max_pct_min,
                    max_pct_max,
                    fee_min,
                    fee_max,
                    threshold_up,
                    threshold_down,
                    initial_base,
                    allowed: allowed.clone(),
                    is_stable,
                    static_fee_bps,
                },
            )?;

            info_pool_cpi::governance_add_asset(
                CpiContext::new_with_signer(
                    ctx.accounts.info_pool_program.to_account_info(),
                    InfoGovernanceAddAsset {
                        info_pool:            ctx.accounts.info_pool_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                    },
                    signer_seeds,
                ),
                mint, max_pct_min, max_pct_max,
                fee_min, fee_max, threshold_up, threshold_down,
                initial_base, allowed, is_stable, static_fee_bps,
            )?;

            emit!(ProposalExecuted {
                pool_id,
                proposal_id:   proposal_id_snap,
                proposal_type: ProposalType::AddAsset,
                description:   format!(
                    "Asset added (Pool + Info Pool): mint={} fee={}–{}bps pct={}%–{}%",
                    mint, fee_min, fee_max, max_pct_min, max_pct_max
                ),
            });
        }

        // ── RemoveAsset ───────────────────────────
        // CPI 1 → Pool:      governance_remove_asset (closes AssetAccount, rent → executor)
        // CPI 2 → Info Pool: governance_remove_asset (drops asset from 3-layer engine)
        // Pool enforces: asset.amount == 0 and mint != base_asset.
        ProposalPayload::RemoveAsset { mint } => {
            pool_cpi::governance_remove_asset(
                CpiContext::new_with_signer(
                    ctx.accounts.pool_program.to_account_info(),
                    GovernanceRemoveAsset {
                        pool:                 ctx.accounts.pool_account.to_account_info(),
                        asset:                ctx.accounts.asset_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                        rent_recipient:       ctx.accounts.executor.to_account_info(),
                    },
                    signer_seeds,
                ),
            )?;

            info_pool_cpi::governance_remove_asset(
                CpiContext::new_with_signer(
                    ctx.accounts.info_pool_program.to_account_info(),
                    InfoGovernanceRemoveAsset {
                        info_pool:            ctx.accounts.info_pool_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                    },
                    signer_seeds,
                ),
                mint,
            )?;

            emit!(ProposalExecuted {
                pool_id,
                proposal_id:   proposal_id_snap,
                proposal_type: ProposalType::RemoveAsset,
                description:   format!("Asset removed (Pool + Info Pool): mint={}", mint),
            });
        }

        // ── UpdateAllowance ───────────────────────
        // CPI 1 → Pool:      governance_set_allowance
        //   Updates the `allowed` Vec on the AssetAccount PDA.
        // CPI 2 → Info Pool: governance_set_allowance
        //   Mirrors the change onto AssetInfo so the Routing program
        //   can filter tradeable pairs from a single InfoPoolAccount
        //   read without a separate Pool cross-program call.
        // `asset`  = source mint whose allowance list is updated.
        // `target` = the mint being added or removed from that list.
        ProposalPayload::UpdateAllowance { asset, target, allowed } => {
            pool_cpi::governance_set_allowance(
                CpiContext::new_with_signer(
                    ctx.accounts.pool_program.to_account_info(),
                    GovernanceSetAllowance {
                        pool:                 ctx.accounts.pool_account.to_account_info(),
                        asset:                ctx.accounts.asset_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                    },
                    signer_seeds,
                ),
                asset, target, allowed,
            )?;

            info_pool_cpi::governance_set_allowance(
                CpiContext::new_with_signer(
                    ctx.accounts.info_pool_program.to_account_info(),
                    InfoGovernanceSetAllowance {
                        info_pool:            ctx.accounts.info_pool_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                    },
                    signer_seeds,
                ),
                asset, target, allowed,
            )?;

            emit!(ProposalExecuted {
                pool_id,
                proposal_id:   proposal_id_snap,
                proposal_type: ProposalType::UpdateAllowance,
                description:   format!(
                    "Allowance updated (Pool + Info Pool): asset={} target={} allowed={}",
                    asset, target, allowed
                ),
            });
        }

        // ── SetPythFeedId ─────────────────────────
        // CPI → Info Pool: governance_set_pyth_feed_id
        //   Rotates the 32-byte Pyth V2 feed ID stored on the AssetInfo.
        //   After execution the crank will use the new feed ID on its
        //   next update_pyth_feeds call; no Pool CPI is needed because
        //   the Pool program has no knowledge of Pyth feed IDs.
        //
        // The Info Pool instruction still accepts the InfoPool founding
        // authority as a bypass signer for pre-governance setup.
        // Once governance is live, this proposal is the canonical path
        // for rotating feed IDs and removes the LP-authority dependency.
        ProposalPayload::SetPythFeedId { mint, pyth_feed_id } => {
            info_pool_cpi::governance_set_pyth_feed_id(
                CpiContext::new_with_signer(
                    ctx.accounts.info_pool_program.to_account_info(),
                    InfoGovernanceSetPythFeedId {
                        info_pool:            ctx.accounts.info_pool_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                    },
                    signer_seeds,
                ),
                mint, pyth_feed_id,
            )?;

            emit!(ProposalExecuted {
                pool_id,
                proposal_id:   proposal_id_snap,
                proposal_type: ProposalType::SetPythFeedId,
                description:   format!(
                    "Pyth feed ID rotated: mint={}",
                    mint
                ),
            });
        }

        // ── SetInflowBlocked ──────────────────────
        // CPI → Pool: governance_set_inflow_blocked
        //   Manually blocks (true) or unblocks (false) inflow for an asset.
        //   No Info Pool CPI needed — Pool owns the is_blocked flag.
        //   Use case: Pool 3 → Public via governance vote on `blocked = false`.
        //   Unblocking also resets threshold_state → Neutral on Pool side.
        ProposalPayload::SetInflowBlocked { mint, blocked } => {
            pool_cpi::governance_set_inflow_blocked(
                CpiContext::new_with_signer(
                    ctx.accounts.pool_program.to_account_info(),
                    GovernanceSetInflowBlocked {
                        pool:                 ctx.accounts.pool_account.to_account_info(),
                        asset:                ctx.accounts.asset_account.to_account_info(),
                        governance_authority: ctx.accounts.governance.to_account_info(),
                    },
                    signer_seeds,
                ),
                mint, blocked,
            )?;

            emit!(ProposalExecuted {
                pool_id,
                proposal_id:   proposal_id_snap,
                proposal_type: ProposalType::SetInflowBlocked,
                description:   format!(
                    "Inflow blocked flag set: mint={} blocked={}",
                    mint, blocked
                ),
            });
        }
    }

    // ── MARK EXECUTED ─────────────────────────────
    proposal.executed = true;
    proposal.status   = ProposalStatus::Executed;

    Ok(())
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct ProposalExecuted {
    pub pool_id:       Pubkey,
    pub proposal_id:   u64,
    pub proposal_type: ProposalType,
    pub description:   String,
}
