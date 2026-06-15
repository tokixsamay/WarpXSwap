use anchor_lang::prelude::*;

#[error_code]
pub enum GovernanceError {
    #[msg("Not in top 10 holders — cannot propose")]
    NotInTop10,

    #[msg("Proposer cooldown active — wait 48 hours")]
    ProposerCooldown,

    #[msg("Proposal is not active for voting")]
    ProposalNotActive,

    #[msg("Voting window has closed")]
    VotingClosed,

    #[msg("Already voted on this proposal")]
    AlreadyVoted,

    #[msg("Proposal has not passed")]
    ProposalNotPassed,

    #[msg("Proposal already executed")]
    AlreadyExecuted,

    #[msg("Not a registered contributor")]
    NotRegistered,

    #[msg("Unauthorized — not pool program")]
    NotPoolProgram,

    #[msg("Unauthorized — not info pool program")]
    NotInfoPoolProgram,

    #[msg("Emergency proposal requires Top 10 approval first")]
    EmergencyNotApproved,

    #[msg("Not an emergency proposal")]
    NotEmergency,

    #[msg("Not in top 10 — cannot approve emergency")]
    NotTop10ForEmergency,

    #[msg("Already approved this emergency proposal")]
    AlreadyApprovedEmergency,

    #[msg("Math overflow")]
    MathOverflow,

    #[msg("Invalid proposal payload")]
    InvalidPayload,

    #[msg("Contributor stake is zero")]
    ZeroStake,

    #[msg("Voting window has not closed yet — cannot finalize")]
    VotingWindowNotClosed,

    #[msg("Proposal is already closed (Passed / Rejected / Executed)")]
    ProposalAlreadyClosed,

    #[msg("Not enough votes cast to reach quorum")]
    InsufficientVotes,

    #[msg("Invalid parameter value")]
    InvalidParameter,

    #[msg("Timelock active — proposal cannot be executed yet; wait for execute_after")]
    TimelockActive,
}
