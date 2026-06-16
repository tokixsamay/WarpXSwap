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
        /// Bug #4 fix: changed from u64 to i64 to match AssetAccount.current_base (i64)
        /// and InfoPool's AssetInfo.current_base (i64).
        initial_base:   i64,
        allowed:        Vec<Pubkey>,
        is_stable:      bool,
        static_fee_bps: u16,
        /// Bug #2 fix: token decimal precision needed for USD normalisation.
        decimals:       u8,
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
    SetPythFeedId {
        mint:         Pubkey,
        pyth_feed_id: [u8; 32],
    },
    /// Manually block (true) or unblock (false) inflow for an asset.
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
    pub pool_id:            Pubkey,
    /// Top-10 wallet keys (sorted by stake in Bug #16 fix — highest first).
    pub top_10:             Vec<Pubkey>,
    /// Bug #16 fix: parallel Vec of stake amounts for top_10.
    /// top_10[i] holds the wallet; top_10_stakes[i] holds its stake.
    /// Kept in sync on every register/update_stake call.
    /// Enables stake-sorted displacement: a new wallet can displace the
    /// lowest-stake entry when the list is full.
    pub top_10_stakes:      Vec<u64>,
    pub total_stake:        u64,
    pub proposal_count:     u64,
    pub last_updated:       i64,
    pub min_votes_to_pass:  u64,
    pub execute_delay_secs: i64,
    pub bump:               u8,
}

impl GovernanceAccount {
    // 8 discriminator + 32 pool_id
    // + 4+320 top_10 (Vec<Pubkey>, max 10)
    // + 4+80  top_10_stakes (Vec<u64>, max 10) ← Bug #16 addition
    // + 8+8+8+8+8 (total_stake, proposal_count, last_updated, min_votes_to_pass, execute_delay_secs)
    // + 1 bump
    // = 8+32+324+84+40+1 = 489 → rounded to 520 for safety
    pub const LEN: usize = 520;
    pub const MAX_TOP_N: usize = 10;
}

// ── CONTRIBUTOR ACCOUNT ───────────────────────────
#[account]
pub struct ContributorAccount {
    pub contributor:   Pubkey,
    pub pool_id:       Pubkey,
    pub stake_amount:  u64,
    pub last_proposal: i64,
    pub voted_on:      Vec<u64>,
    pub is_top_10:     bool,
    pub bump:          u8,
}

impl ContributorAccount {
    pub const LEN: usize = 8 + 32 + 32 + 8 + 8 + 4 + (8 * 50) + 1 + 1;
    pub const MAX_VOTED: usize = 50;
}

// ── PROPOSAL ACCOUNT ──────────────────────────────
#[account]
pub struct ProposalAccount {
    pub proposal_id:          u64,
    pub proposer:             Pubkey,
    pub pool_id:              Pubkey,
    pub proposal_type:        ProposalType,
    pub payload:              ProposalPayload,
    pub status:               ProposalStatus,
    pub votes_yes:            u64,
    pub votes_no:             u64,
    pub created_at:           i64,
    pub ends_at:              i64,
    pub is_emergency:         bool,
    pub emergency_approvals:  Vec<Pubkey>,
    pub executed:             bool,
    pub execute_after:        i64,
    pub bump:                 u8,
}

impl ProposalAccount {
    // payload: largest variant is AddAsset =
    //   1 (discriminant) + 32 (mint) + 1 + 1 + 2 + 2 + 2 + 2 + 8 (initial_base, i64)
    //   + 4 + (32*10) (allowed Vec, max 10) + 1 (is_stable) + 2 (static_fee_bps)
    //   + 1 (decimals, Bug #2 addition) = 376 bytes; padded to 380.
    // 8 + 8 + 32 + 32 + 2 + 380 + 2 + 8 + 8 + 8 + 8 + 1 + 4+(32*10) + 1 + 1 + 8 (execute_after) + 1 (bump)
    pub const LEN: usize = 8 + 8 + 32 + 32 + 2 + 380 + 2 + 8 + 8 + 8 + 8 + 1 + 4 + (32 * 10) + 1 + 1 + 8 + 1;
}
