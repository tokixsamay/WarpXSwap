use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::GovernanceError;

// ═══════════════════════════════════════════════════
// CAST VOTE
// All LP contributors can vote
// Vote weight = equal (1 vote per contributor)
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(proposal_id: u64, vote: bool)]
pub struct CastVote<'info> {
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

    #[account(
        mut,
        seeds = [
            CONTRIBUTOR_SEED,
            governance.pool_id.as_ref(),
            voter.key().as_ref()
        ],
        bump = contributor.bump,
    )]
    pub contributor: Account<'info, ContributorAccount>,

    pub voter: Signer<'info>,
}

pub fn handler(
    ctx: Context<CastVote>,
    proposal_id: u64,
    vote: bool,
) -> Result<()> {
    let gov         = &ctx.accounts.governance;
    let proposal    = &mut ctx.accounts.proposal;
    let contributor = &mut ctx.accounts.contributor;
    let voter       = ctx.accounts.voter.key();
    let now         = Clock::get()?.unix_timestamp;

    // ── CHECKS ────────────────────────────────────

    // Proposal must be active
    require!(
        proposal.status == ProposalStatus::Active,
        GovernanceError::ProposalNotActive
    );

    // Voting window open
    require!(
        now <= proposal.ends_at,
        GovernanceError::VotingClosed
    );

    // Must be registered contributor
    require!(
        contributor.stake_amount > 0,
        GovernanceError::NotRegistered
    );

    // ── EMERGENCY GATE ────────────────────────────
    // Emergency proposals were already pre-screened by a Top 10 majority
    // in approve_emergency. Restrict voting to Top 10 only so that the
    // same insider group that escalated the proposal is also the one that
    // ratifies it — preventing a community vote from overriding it quickly.
    if proposal.is_emergency {
        require!(
            contributor.is_top_10,
            GovernanceError::NotInTop10
        );
        require!(
            gov.top_10.contains(&voter),
            GovernanceError::NotInTop10
        );
    }

    // No double vote
    require!(
        !contributor.voted_on.contains(&proposal_id),
        GovernanceError::AlreadyVoted
    );

    // ── RECORD VOTE ───────────────────────────────
    // Equal vote weight (1 per contributor)
    let vote_weight: u64 = 1;

    if vote {
        proposal.votes_yes = proposal.votes_yes
            .checked_add(vote_weight)
            .ok_or(GovernanceError::MathOverflow)?;
    } else {
        proposal.votes_no = proposal.votes_no
            .checked_add(vote_weight)
            .ok_or(GovernanceError::MathOverflow)?;
    }

    // Track voted proposals (prevent double vote)
    // Prune proposals older than the rolling window instead of rotating,
    // so a voter cannot re-vote on an old proposal after eviction.
    if contributor.voted_on.len() >= ContributorAccount::MAX_VOTED {
        let min_valid_id = proposal.proposal_id
            .saturating_sub(ContributorAccount::MAX_VOTED as u64);
        contributor.voted_on.retain(|&id| id >= min_valid_id);
    }
    contributor.voted_on.push(proposal_id);

    // ── CHECK QUORUM ──────────────────────────────
    // Count total votes cast. We only attempt to mark the proposal Passed
    // once gov.min_votes_to_pass have been cast. Every vote is always recorded;
    // quorum just gates the status transition.
    let total_votes = proposal.votes_yes
        .checked_add(proposal.votes_no)
        .ok_or(GovernanceError::MathOverflow)?;

    // We only attempt to mark the proposal Passed once gov.min_votes_to_pass
    // have been cast. We do NOT revert here if the minimum is not yet met —
    // doing so would prevent voters 1 … (min_votes_to_pass - 1) from ever
    // recording their vote (the TX would revert before state is committed).
    // Every vote is always recorded; quorum just gates the status transition.
    // min_votes_to_pass is set per-pool at governance creation time.
    if total_votes >= gov.min_votes_to_pass {
        // 51% of YES votes from total votes cast
        // (not from total stake — equal weight per voter)
        let yes_bps = proposal.votes_yes
            .checked_mul(BPS_DENOMINATOR)
            .ok_or(GovernanceError::MathOverflow)?
            .checked_div(total_votes.max(1))
            .ok_or(GovernanceError::MathOverflow)?;

        if yes_bps >= QUORUM_BPS {
            proposal.status       = ProposalStatus::Passed;
            // Timelock: executor cannot run until execute_delay_secs have elapsed.
            // This is a per-governance runtime value set at creation — never a
            // compile-time constant — so a mainnet binary cannot accidentally
            // ship with a 0-second timelock.
            proposal.execute_after = now + gov.execute_delay_secs;
            emit!(ProposalPassed {
                pool_id:     gov.pool_id,
                proposal_id,
                votes_yes:   proposal.votes_yes,
                votes_no:    proposal.votes_no,
            });
        }
    }

    emit!(VoteCast {
        pool_id:     gov.pool_id,
        proposal_id,
        voter,
        vote,
        votes_yes:   proposal.votes_yes,
        votes_no:    proposal.votes_no,
    });

    Ok(())
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct VoteCast {
    pub pool_id:     Pubkey,
    pub proposal_id: u64,
    pub voter:       Pubkey,
    pub vote:        bool,
    pub votes_yes:   u64,
    pub votes_no:    u64,
}

#[event]
pub struct ProposalPassed {
    pub pool_id:     Pubkey,
    pub proposal_id: u64,
    pub votes_yes:   u64,
    pub votes_no:    u64,
  }
      
