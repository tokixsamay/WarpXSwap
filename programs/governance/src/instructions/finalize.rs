use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::GovernanceError;

// ═══════════════════════════════════════════════════
// FINALIZE PROPOSAL
// Anyone can call this after a proposal's voting window
// has closed.  If the proposal is still Active (meaning
// it never crossed the 51% YES threshold during voting),
// it is marked Rejected on-chain.
//
// When a proposal DOES reach 51% during cast_vote, vote.rs
// transitions it to Passed immediately — so a proposal
// sitting at Active after ends_at always means quorum was
// never reached.
//
// The govern-crank calls this automatically to close out
// expired proposals rather than leaving them in Active state.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(proposal_id: u64)]
pub struct FinalizeProposal<'info> {
    #[account(
        seeds = [GOVERNANCE_SEED, governance.pool_id.as_ref()],
        bump   = governance.bump,
    )]
    pub governance: Account<'info, GovernanceAccount>,

    #[account(
        mut,
        seeds = [
            PROPOSAL_SEED,
            governance.pool_id.as_ref(),
            &proposal_id.to_le_bytes(),
        ],
        bump = proposal.bump,
    )]
    pub proposal: Account<'info, ProposalAccount>,

    /// Anyone can finalize — no special authority required.
    /// The instruction is purely a state transition based on
    /// on-chain time; no privileged action is taken.
    pub caller: Signer<'info>,
}

pub fn handler(
    ctx: Context<FinalizeProposal>,
    _proposal_id: u64,
) -> Result<()> {
    let proposal = &mut ctx.accounts.proposal;
    let pool_id  = ctx.accounts.governance.pool_id;
    let now      = Clock::get()?.unix_timestamp;

    // Only Active proposals can be finalized
    require!(
        proposal.status == ProposalStatus::Active,
        GovernanceError::ProposalAlreadyClosed
    );

    // Voting window must have actually closed
    require!(
        now > proposal.ends_at,
        GovernanceError::VotingWindowNotClosed
    );

    // At this point: Active after deadline = quorum never reached → Rejected
    proposal.status = ProposalStatus::Rejected;

    let total = proposal.votes_yes
        .checked_add(proposal.votes_no)
        .unwrap_or(0);
    let yes_bps = if total > 0 {
        proposal.votes_yes
            .checked_mul(BPS_DENOMINATOR)
            .unwrap_or(0)
            .checked_div(total)
            .unwrap_or(0)
    } else {
        0
    };

    emit!(ProposalRejected {
        pool_id,
        proposal_id: proposal.proposal_id,
        votes_yes:   proposal.votes_yes,
        votes_no:    proposal.votes_no,
        yes_bps,
        ended_at:    proposal.ends_at,
    });

    Ok(())
}

// ── EVENT ─────────────────────────────────────────
#[event]
pub struct ProposalRejected {
    pub pool_id:     Pubkey,
    pub proposal_id: u64,
    pub votes_yes:   u64,
    pub votes_no:    u64,
    /// Final YES percentage in basis points (e.g. 4800 = 48.00%)
    pub yes_bps:     u64,
    /// Timestamp when the voting window closed
    pub ended_at:    i64,
}
