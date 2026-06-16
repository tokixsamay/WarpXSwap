use anchor_lang::prelude::*;

pub mod instructions;
pub mod state;
pub mod errors;
pub mod constants;

#[cfg(test)]
mod tests;

use instructions::*;

declare_id!("C1iFRYB3fw7Rq2i2JFruYLbJoGTxRb6ohYqerYBpUsLm");

#[program]
pub mod governance_program {
    use super::*;

    // ── SETUP ─────────────────────────────────────

    pub fn initialize_governance(
        ctx: Context<InitializeGovernance>,
        pool_id:            Pubkey,
        min_votes_to_pass:  u64,
        execute_delay_secs: i64,
    ) -> Result<()> {
        instructions::initialize::handler(ctx, pool_id, min_votes_to_pass, execute_delay_secs)
    }

    pub fn register_contributor(
        ctx: Context<RegisterContributor>,
        stake_amount: u64,
    ) -> Result<()> {
        instructions::initialize::handler_register(ctx, stake_amount)
    }

    pub fn update_contributor_stake(
        ctx: Context<UpdateContributorStake>,
        new_stake: u64,
    ) -> Result<()> {
        instructions::initialize::handler_update_stake(ctx, new_stake)
    }

    // ── PROPOSALS ─────────────────────────────────

    pub fn create_proposal(
        ctx: Context<CreateProposal>,
        proposal_type: ProposalType,
        payload: ProposalPayload,
        is_emergency: bool,
    ) -> Result<()> {
        instructions::proposal::handler_create(
            ctx,
            proposal_type,
            payload,
            is_emergency,
        )
    }

    pub fn approve_emergency(
        ctx: Context<ApproveEmergency>,
        proposal_id: u64,
    ) -> Result<()> {
        instructions::proposal::handler_approve_emergency(ctx, proposal_id)
    }

    // ── VOTING ────────────────────────────────────

    pub fn cast_vote(
        ctx: Context<CastVote>,
        proposal_id: u64,
        vote: bool,
    ) -> Result<()> {
        instructions::vote::handler(ctx, proposal_id, vote)
    }

    // ── EXECUTION ─────────────────────────────────

    pub fn execute_proposal(
        ctx: Context<ExecuteProposal>,
        proposal_id: u64,
    ) -> Result<()> {
        instructions::execute::handler(ctx, proposal_id)
    }

    // ── FINALIZATION ──────────────────────────────
    // Marks an Active proposal as Rejected once its voting
    // window has closed without reaching 51% YES quorum.
    // Anyone can call this — the govern-crank does so automatically.

    pub fn finalize_proposal(
        ctx: Context<FinalizeProposal>,
        proposal_id: u64,
    ) -> Result<()> {
        instructions::finalize::handler(ctx, proposal_id)
    }
      }
