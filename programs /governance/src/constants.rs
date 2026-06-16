/// Seeds
pub const GOVERNANCE_SEED:   &[u8] = b"governance";
pub const CONTRIBUTOR_SEED:  &[u8] = b"contributor";
pub const PROPOSAL_SEED:     &[u8] = b"proposal";

/// Voting window: 48 hours in seconds
pub const VOTING_WINDOW_SECS: i64 = 48 * 60 * 60;

/// Timelock delay between a proposal passing and execution.
/// Gives LPs time to react (exit, object) before an on-chain change is applied.
///
/// Bug #18 fix: renamed from EXECUTE_DELAY_SECS (was 0) to make the
/// devnet-vs-mainnet distinction explicit. Using the wrong constant at
/// mainnet deployment would allow zero-timelock governance actions.
///
/// DEVNET — 0 so tests can execute immediately.
pub const DEVNET_EXECUTE_DELAY_SECS: i64 = 0;
/// MAINNET — 24 hours minimum. Use 48 * 60 * 60 for stricter pools.
pub const MAINNET_EXECUTE_DELAY_SECS: i64 = 24 * 60 * 60;
/// Minimum acceptable execute_delay_secs for a mainnet deployment.
/// The SDK setup script should enforce this when calling governance_initialize.
pub const MIN_MAINNET_EXECUTE_DELAY_SECS: i64 = 86_400; // 24 h

/// Proposer cooldown: 48 hours in seconds
pub const PROPOSER_COOLDOWN_SECS: i64 = 48 * 60 * 60;

/// Quorum: 51% of total stake
pub const QUORUM_BPS: u64 = 5_100;

/// Basis points denominator
pub const BPS_DENOMINATOR: u64 = 10_000;

/// Bug #15A fix: mirror Pool's MAX_FEE_BPS / MIN_FEE_BPS so validate_payload
/// and governance_update_fee_range use identical bounds and can never diverge.
/// These MUST be kept in sync with pool/src/constants.rs.
pub const MAX_FEE_BPS: u16 = 500;
pub const MIN_FEE_BPS: u16 = 1;

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

/// Bug #16 fix: minimum stake required for top-10 governance eligibility.
/// Prevents zero-stake wallets from occupying top-10 slots permanently.
/// Expressed in the pool's base-asset native units (e.g. 1 SOL = 1_000_000_000
/// lamports; adjust per pool at init time if needed via MIN_VOTES_TO_PASS).
/// At 1_000_000, a wallet must hold ≥1 unit of the smallest 6-decimal token.
pub const MIN_STAKE_FOR_TOP10: u64 = 1_000_000;
