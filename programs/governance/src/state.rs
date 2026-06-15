use anchor_lang::prelude::*;

// ── PROPOSAL TYPE ──────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, Debug)]
pub enum ProposalType {
    AddAsset,
    RemoveAsset,
    UpdateAllowance,
    UpdateMaxPct,
    /// Combined up+down threshold update (replaces the old separate Up/Down variants).
    UpdateThreshold,
    UpdateFeeRange,
    /// Rotate the Pyth V2 feed ID for an asset without LP authority.
    SetPythFeedId,
    /// Manually block or unblock inflow for an asset (emergency circuit-breaker).
    SetInflowBlocked,
}

// ── PROPOSAL PAYLOAD ──────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub enum ProposalPayload {
    AddAsset {
        mint:           Pubkey,
        max_pct_min:    u8,
        max_pct_max:    u8,
        fee_min:        u16,
        fee_max:        u16,
        threshold_up:   u16,
        threshold_down: u16,
        initial_base:   u64,
        allowed:        Vec<Pubkey>,
        is_stable:      bool,
        static_fee_bps: u16,
    },
    RemoveAsset {
        mint: Pubkey,
    },
    UpdateAllowance {
        asset:      Pubkey,
        target:     Pubkey,
        allowed:    bool,
    },
    UpdateMaxPct {
        mint:    Pubkey,
        new_min: u8,
        new_max: u8,
    },
    UpdateThreshold {
        mint:     Pubkey,
        new_up:   u16,
        new_down: u16,
    },
    UpdateFeeRange {
        mint:    Pubkey,
        new_min: u16,
        new_max: u16,
    },
    /// Rotate the Pyth V2 feed ID stored on an AssetInfo.
    /// Payload: 1 (discriminant) + 32 (mint) + 32 (pyth_feed_id) = 65 bytes.
    SetPythFeedId {
        mint:         Pubkey,
        pyth_feed_id: [u8; 32],
    },
    /// Manually block (true) or unblock (false) inflow for an asset.
    /// Unblocking also resets threshold_state → Neutral on the Pool side.
    /// Use as an emergency circuit-breaker (e.g. Pool 3 → Public via governance).
    SetInflowBlocked {
        mint:    Pubkey,
        blocked: bool,
    },
}

// ── PROPOSAL STATUS ────────────────────────────────
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Eq, Debug)]
pub enum ProposalStatus {
    /// Emergency: waiting for Top 10 majority approval
    PendingEmergencyApproval,
    /// Open for community voting
    Active,
    /// 51% yes votes reached — pending execution
    Passed,
    /// Failed to reach quorum
    Rejected,
    /// Changes applied on-chain
    Executed,
}

// ── GOVERNANCE ACCOUNT ────────────────────────────
#[account]
pub struct GovernanceAccount {
    /// Associated pool
    pub pool_id:            Pubkey,
    /// Top 10 holders (updated dynamically)
    pub top_10:             Vec<Pubkey>,
    /// Total voting power (sum of all stakes)
    pub total_stake:        u64,
    /// Total proposals created
    pub proposal_count:     u64,
    /// Last updated timestamp
    pub last_updated:       i64,
    /// Minimum total votes that must be cast before a proposal can be
    /// marked Passed. Set at governance creation; governs this pool only.
    /// Replaces the old compile-time MIN_VOTES_TO_PASS constant so each
    /// pool can choose its own participation threshold.
    pub min_votes_to_pass:  u64,
    /// Seconds after a proposal passes before it may be executed (timelock).
    /// Stored per-governance so each pool can tune its own safety window.
    /// Set to 0 on devnet for immediate execution; use ≥ 86_400 (24 h) on mainnet.
    /// Replaces the old compile-time EXECUTE_DELAY_SECS constant so a mainnet
    /// binary cannot accidentally ship with a 0-second timelock.
    pub execute_delay_secs: i64,
    /// PDA bump
    pub bump:               u8,
}

impl GovernanceAccount {
    // 8 discriminator + 32 pool_id + (4 + 32*10) top_10 + 8 total_stake
    // + 8 proposal_count + 8 last_updated + 8 min_votes_to_pass
    // + 8 execute_delay_secs + 1 bump
    pub const LEN: usize = 8 + 32 + 4 + (32 * 10) + 8 + 8 + 8 + 8 + 8 + 1;
    pub const MAX_TOP_N: usize = 10;
}

// ── CONTRIBUTOR ACCOUNT ───────────────────────────
#[account]
pub struct ContributorAccount {
    /// Contributor wallet
    pub contributor:   Pubkey,
    /// Associated pool
    pub pool_id:       Pubkey,
    /// Stake amount (= LP deposit amount)
    pub stake_amount:  u64,
    /// Last proposal timestamp (cooldown check)
    pub last_proposal: i64,
    /// Proposals voted on (prevent double vote)
    pub voted_on:      Vec<u64>,
    /// Is in top 10?
    pub is_top_10:     bool,
    /// PDA bump
    pub bump:          u8,
}

impl ContributorAccount {
    // 8 + 32 + 32 + 8 + 8 + 4 + (8*50) + 1 + 1
    // voted_on: max 50 recent proposals tracked
    pub const LEN: usize = 8 + 32 + 32 + 8 + 8 + 4 + (8 * 50) + 1 + 1;
    pub const MAX_VOTED: usize = 50;
}

// ── PROPOSAL ACCOUNT ──────────────────────────────
#[account]
pub struct ProposalAccount {
    /// Unique proposal ID
    pub proposal_id:          u64,
    /// Who created it
    pub proposer:             Pubkey,
    /// Associated pool
    pub pool_id:              Pubkey,
    /// Proposal type
    pub proposal_type:        ProposalType,
    /// Proposal data
    pub payload:              ProposalPayload,
    /// Current status
    pub status:               ProposalStatus,
    /// Yes votes (stake-weighted)
    pub votes_yes:            u64,
    /// No votes (stake-weighted)
    pub votes_no:             u64,
    /// Creation timestamp
    pub created_at:           i64,
    /// Voting ends at
    pub ends_at:              i64,
    /// Emergency proposal flag
    pub is_emergency:         bool,
    /// Top 10 approvals for emergency
    pub emergency_approvals:  Vec<Pubkey>,
    /// Has been executed
    pub executed:             bool,
    /// Earliest timestamp at which this proposal may be executed (timelock).
    /// Set to `Clock::unix_timestamp + EXECUTE_DELAY_SECS` when status → Passed.
    /// Zero means no delay (devnet default; see EXECUTE_DELAY_SECS constant).
    pub execute_after:        i64,
    /// PDA bump
    pub bump:                 u8,
}

impl ProposalAccount {
    // payload: largest variant is AddAsset =
    //   1 (discriminant) + 32 (mint) + 1 + 1 + 2 + 2 + 2 + 2 + 8 (initial_base)
    //   + 4 + (32*10) (allowed Vec, max 10) = 375 bytes; use 376 with padding
    // 8 + 8 + 32 + 32 + 2 + 376 + 2 + 8 + 8 + 8 + 8 + 1 + 4+(32*10) + 1 + 1 + 8 (execute_after)
    pub const LEN: usize = 8 + 8 + 32 + 32 + 2 + 376 + 2 + 8 + 8 + 8 + 8 + 1 + 4 + (32 * 10) + 1 + 1 + 8;
}
