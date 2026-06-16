use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::GovernanceError;
use crate::instructions::initialize::update_top_10;

// ═══════════════════════════════════════════════════
// CREATE PROPOSAL
// Only top 10 holders can propose
// 48-hour cooldown per proposer
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct CreateProposal<'info> {
    #[account(
        mut,
        seeds = [GOVERNANCE_SEED, governance.pool_id.as_ref()],
        bump = governance.bump,
    )]
    pub governance: Account<'info, GovernanceAccount>,

    #[account(
        mut,
        seeds = [
            CONTRIBUTOR_SEED,
            governance.pool_id.as_ref(),
            proposer.key().as_ref()
        ],
        bump = contributor.bump,
    )]
    pub contributor: Account<'info, ContributorAccount>,

    #[account(
        init,
        payer = proposer,
        space = ProposalAccount::LEN,
        seeds = [
            PROPOSAL_SEED,
            governance.pool_id.as_ref(),
            &governance.proposal_count.to_le_bytes()
        ],
        bump
    )]
    pub proposal: Account<'info, ProposalAccount>,

    #[account(mut)]
    pub proposer: Signer<'info>,

    pub system_program: Program<'info, System>,
}

/// Validates the payload fields before creating a proposal.
/// Catches clearly invalid parameters early so proposer funds are not wasted
/// on a proposal that will always revert at execution time.
fn validate_payload(payload: &ProposalPayload) -> Result<()> {
    match payload {
        ProposalPayload::UpdateFeeRange { new_min, new_max, .. } => {
            require!(
                new_min < new_max,
                GovernanceError::InvalidParameter
            );
            // Bug #15A fix: match Pool's handler_update_fee_range bounds exactly.
            // Pool enforces new_max <= MAX_FEE_BPS (500) and new_min >= MIN_FEE_BPS (1).
            // The old check used BPS_DENOMINATOR (10_000), so any proposal with
            // new_max in 501–10_000 passed validation but always reverted at execution
            // — permanently stuck in Passed state.
            require!(
                *new_min >= MIN_FEE_BPS,
                GovernanceError::InvalidParameter
            );
            require!(
                *new_max <= MAX_FEE_BPS,
                GovernanceError::InvalidParameter
            );
        }
        ProposalPayload::UpdateThreshold { new_up, new_down, .. } => {
            require!(
                *new_up > 0 && *new_down > 0,
                GovernanceError::InvalidParameter
            );
        }
        ProposalPayload::UpdateMaxPct { new_min, new_max, .. } => {
            require!(
                new_min < new_max,
                GovernanceError::InvalidParameter
            );
            require!(
                (*new_max as u64) <= 100,
                GovernanceError::InvalidParameter
            );
        }
        ProposalPayload::AddAsset { fee_min, fee_max, threshold_up, threshold_down,
                                    max_pct_min, max_pct_max, is_stable, initial_base, .. } => {
            // Bug #15B fix: execution uses strict `<` (max_pct_min < max_pct_max).
            // The old `<=` allowed equal values that then reverted at execution.
            require!(
                max_pct_min < max_pct_max,
                GovernanceError::InvalidParameter
            );
            // Bug #15C fix: InfoPool's handler_add_asset requires initial_base > 0.
            // Without this check, a proposal with initial_base = 0 would pass the
            // vote then revert on execution — permanently stuck in Passed state.
            require!(
                *initial_base > 0,
                GovernanceError::InvalidParameter
            );
            if *is_stable {
                // Stablecoins use a fixed fee; no threshold blocking logic.
                require!(
                    *threshold_up == 0 && *threshold_down == 0,
                    GovernanceError::InvalidParameter
                );
            } else {
                require!(
                    fee_min < fee_max,
                    GovernanceError::InvalidParameter
                );
                require!(
                    *threshold_up > 0 && *threshold_down > 0,
                    GovernanceError::InvalidParameter
                );
            }
        }
        // Other variants have no numeric invariants to enforce at proposal time.
        _ => {}
    }
    Ok(())
}

pub fn handler_create(
    ctx: Context<CreateProposal>,
    proposal_type: ProposalType,
    payload: ProposalPayload,
    is_emergency: bool,
) -> Result<()> {
    let gov         = &mut ctx.accounts.governance;
    let contributor = &mut ctx.accounts.contributor;
    let proposal    = &mut ctx.accounts.proposal;
    let proposer    = ctx.accounts.proposer.key();
    let now         = Clock::get()?.unix_timestamp;

    // ── PAYLOAD VALIDATION ────────────────────────
    // Reject proposals with obviously invalid numeric parameters up-front.
    // This catches fee_min >= fee_max, zero thresholds, etc. before any
    // state is mutated — saves proposer rent and avoids no-op proposals.
    validate_payload(&payload)?;

    // ── AUTH: Must be top 10 ──────────────────────
    require!(
        gov.top_10.contains(&proposer),
        GovernanceError::NotInTop10
    );

    // ── COOLDOWN CHECK ────────────────────────────
    if contributor.last_proposal > 0 {
        let elapsed = now - contributor.last_proposal;
        require!(
            elapsed >= PROPOSER_COOLDOWN_SECS,
            GovernanceError::ProposerCooldown
        );
    }

    // ── CREATE PROPOSAL ───────────────────────────
    let proposal_id = gov.proposal_count;

    proposal.proposal_id         = proposal_id;
    proposal.proposer            = proposer;
    proposal.pool_id             = gov.pool_id;
    proposal.proposal_type       = proposal_type;
    proposal.payload             = payload;
    proposal.votes_yes           = 0;
    proposal.votes_no            = 0;
    proposal.created_at          = now;
    proposal.is_emergency        = is_emergency;
    proposal.emergency_approvals = Vec::new();
    proposal.executed            = false;
    proposal.execute_after       = 0; // Set to now + governance.execute_delay_secs when Passed (vote.rs)
    proposal.bump                = ctx.bumps.proposal;

    if is_emergency {
        // Emergency: pending top 10 approval first
        proposal.status  = ProposalStatus::PendingEmergencyApproval;
        proposal.ends_at = 0; // Set when approved
    } else {
        // Normal: immediately active
        proposal.status  = ProposalStatus::Active;
        proposal.ends_at = now + VOTING_WINDOW_SECS;
    }

    // ── UPDATE STATE ──────────────────────────────
    contributor.last_proposal = now;
    gov.proposal_count = gov.proposal_count
        .checked_add(1)
        .ok_or(GovernanceError::MathOverflow)?;

    emit!(ProposalCreated {
        pool_id:      gov.pool_id,
        proposal_id,
        proposer,
        proposal_type: proposal.proposal_type.clone(),
        is_emergency,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// APPROVE EMERGENCY PROPOSAL
// Top 10 members approve before it goes public
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct ApproveEmergency<'info> {
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

    pub approver: Signer<'info>,
}

pub fn handler_approve_emergency(
    ctx: Context<ApproveEmergency>,
    _proposal_id: u64,
) -> Result<()> {
    let gov      = &ctx.accounts.governance;
    let proposal = &mut ctx.accounts.proposal;
    let approver = ctx.accounts.approver.key();
    let now      = Clock::get()?.unix_timestamp;

    // ── CHECKS ────────────────────────────────────
    require!(
        proposal.is_emergency,
        GovernanceError::NotEmergency
    );

    require!(
        proposal.status == ProposalStatus::PendingEmergencyApproval,
        GovernanceError::ProposalNotActive
    );

    require!(
        gov.top_10.contains(&approver),
        GovernanceError::NotTop10ForEmergency
    );

    require!(
        !proposal.emergency_approvals.contains(&approver),
        GovernanceError::AlreadyApprovedEmergency
    );

    // ── ADD APPROVAL ──────────────────────────────
    proposal.emergency_approvals.push(approver);

    // ── CHECK MAJORITY ────────────────────────────
    let approvals_count = proposal.emergency_approvals.len();
    let top_10_count    = gov.top_10.len();
    let majority_needed = (top_10_count / 2) + 1;

    if approvals_count >= majority_needed {
        // Majority reached → make proposal public
        proposal.status  = ProposalStatus::Active;
        proposal.ends_at = now + VOTING_WINDOW_SECS;

        emit!(EmergencyProposalPublished {
            pool_id:     gov.pool_id,
            proposal_id: proposal.proposal_id,
            approvals:   approvals_count as u8,
        });
    } else {
        emit!(EmergencyApprovalAdded {
            pool_id:         gov.pool_id,
            proposal_id:     proposal.proposal_id,
            approver,
            approvals_count: approvals_count as u8,
            needed:          majority_needed as u8,
        });
    }

    Ok(())
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct ProposalCreated {
    pub pool_id:       Pubkey,
    pub proposal_id:   u64,
    pub proposer:      Pubkey,
    pub proposal_type: ProposalType,
    pub is_emergency:  bool,
}

#[event]
pub struct EmergencyApprovalAdded {
    pub pool_id:         Pubkey,
    pub proposal_id:     u64,
    pub approver:        Pubkey,
    pub approvals_count: u8,
    pub needed:          u8,
}

#[event]
pub struct EmergencyProposalPublished {
    pub pool_id:     Pubkey,
    pub proposal_id: u64,
    pub approvals:   u8,
              }
              
