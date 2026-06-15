/// Seeds
pub const GOVERNANCE_SEED:   &[u8] = b"governance";
pub const CONTRIBUTOR_SEED:  &[u8] = b"contributor";
pub const PROPOSAL_SEED:     &[u8] = b"proposal";

/// Voting window: 48 hours in seconds
pub const VOTING_WINDOW_SECS: i64 = 48 * 60 * 60;

/// Timelock: minimum delay between a proposal passing and execution.
/// Gives LPs time to react (exit, object) before an on-chain change is applied.
/// ⚠  DEVNET: set to 0 so tests can execute immediately.
///    MAINNET: set to at least 24 * 60 * 60 (24 h) or 48 * 60 * 60 (48 h).
pub const EXECUTE_DELAY_SECS: i64 = 0;

/// Proposer cooldown: 48 hours in seconds
pub const PROPOSER_COOLDOWN_SECS: i64 = 48 * 60 * 60;

/// Quorum: 51% of total stake
pub const QUORUM_BPS: u64 = 5_100;

/// Basis points denominator
pub const BPS_DENOMINATOR: u64 = 10_000;

/// Pool Program ID
pub const POOL_PROGRAM_ID: &str =
    "4AXtXF5VWeWKLqP6vHKPpjoc7wQ8r4duDqZ4CENtzsqZ";

/// Info Pool Program ID
pub const INFO_POOL_PROGRAM_ID: &str =
    "9MXoZpzQZzvURN1S1EARJLaDhFuGw3RAppQMYvGTcmPo";

/// Emergency: needs majority of top 10
/// 6 of 10 = 60% majority
pub const EMERGENCY_MAJORITY: usize = 6;

/// Default minimum-votes threshold used by the setup script when initialising
/// a GovernanceAccount. The authoritative runtime value is
/// `GovernanceAccount::min_votes_to_pass` (set at creation and checked in
/// vote.rs); this constant is only a convenience default.
///
/// ⚠  DEVNET: 1 lets 2-LP test pools pass proposals.
///    MAINNET: pass a higher value (e.g. 3) to `governance_initialize`.
pub const DEFAULT_MIN_VOTES_TO_PASS: u64 = 1;
