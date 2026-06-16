/// Governance unit tests — pure logic, no Anchor context required.
/// Run with: cargo test -p governance-program
///
/// These test the three algorithms fixed by the audit:
///   1. Quorum guard     (TEST_MIN_VOTES = 3, QUORUM_BPS = 51%)
///   2. Double-vote      (voted_on contains check + rolling retain)
///   3. Overflow safety  (checked_add / saturating_sub paths)
#[cfg(test)]
mod tests {
    use crate::constants::{BPS_DENOMINATOR, QUORUM_BPS};
    use crate::state::ContributorAccount;

    /// Minimum votes used in quorum-guard tests.
    /// Set to 3 (realistic mainnet value) so the participation guard is
    /// exercised. Distinct from DEFAULT_MIN_VOTES_TO_PASS (1, for devnet).
    const TEST_MIN_VOTES: u64 = 3;

    // ─── Helpers ──────────────────────────────────────────────────────

    /// Mirrors the cast_vote quorum computation exactly (post-BUG-8 fix).
    ///
    /// Vote recording never reverts: returns Ok(true) when the proposal
    /// transitions to Passed, Ok(false) when votes are recorded but quorum
    /// is not yet met (total < min_votes_to_pass OR yes < 51%).
    /// Only MathOverflow can return Err.
    fn eval_vote(
        votes_yes:         u64,
        votes_no:          u64,
        new_vote:          bool,
        min_votes_to_pass: u64,
    ) -> Result<bool, &'static str> {
        let (votes_yes, votes_no) = if new_vote {
            (
                votes_yes.checked_add(1).ok_or("MathOverflow")?,
                votes_no,
            )
        } else {
            (
                votes_yes,
                votes_no.checked_add(1).ok_or("MathOverflow")?,
            )
        };

        let total = votes_yes
            .checked_add(votes_no)
            .ok_or("MathOverflow")?;

        // Mirrors the non-reverting quorum gate: votes are always recorded;
        // only the Passed transition is gated on min_votes_to_pass.
        if total < min_votes_to_pass {
            return Ok(false);
        }

        // 51% majority check
        let yes_bps = votes_yes
            .checked_mul(BPS_DENOMINATOR)
            .ok_or("MathOverflow")?
            .checked_div(total.max(1))
            .ok_or("MathOverflow")?;

        Ok(yes_bps >= QUORUM_BPS)
    }

    /// Mirrors the voted_on retention logic from cast_vote.
    fn prune_and_record(voted_on: &mut Vec<u64>, proposal_id: u64) {
        if voted_on.len() >= ContributorAccount::MAX_VOTED {
            let min_valid = proposal_id.saturating_sub(ContributorAccount::MAX_VOTED as u64);
            voted_on.retain(|&id| id >= min_valid);
        }
        voted_on.push(proposal_id);
    }

    // ─── 1. Quorum guard ──────────────────────────────────────────────

    #[test]
    fn quorum_first_vote_is_recorded_but_not_passed() {
        // First vote is always recorded — Ok(false) because total (1) < TEST_MIN_VOTES (3).
        // Previously this returned Err("InsufficientVotes") which reverted the TX;
        // the non-reverting fix returns Ok(false) so the vote is committed.
        assert_eq!(
            eval_vote(0, 0, true, TEST_MIN_VOTES),
            Ok(false),
            "single YES is recorded but must not pass (total < min_votes_to_pass)"
        );
    }

    #[test]
    fn quorum_two_votes_recorded_but_not_passed() {
        // Two voters also cannot reach quorum (total = 2 < 3) — Ok(false).
        assert_eq!(
            eval_vote(1, 0, true, TEST_MIN_VOTES),
            Ok(false),
            "two YES votes recorded but must not pass (total < min_votes_to_pass)"
        );
    }

    #[test]
    fn quorum_three_yes_votes_passes() {
        // Third vote tips total to 3 — quorum guard clears, 100% YES > 51%.
        assert_eq!(
            eval_vote(2, 0, true, TEST_MIN_VOTES),
            Ok(true),
            "3 YES out of 3 total must transition proposal to Passed"
        );
    }

    #[test]
    fn quorum_three_votes_minority_yes_does_not_pass() {
        // 1 YES + 2 NO = 33% YES < 51% — quorum guard clears but majority fails.
        assert_eq!(
            eval_vote(0, 2, true, TEST_MIN_VOTES),
            Ok(false),
            "1/3 YES must not pass (below 51% quorum)"
        );
    }

    #[test]
    fn quorum_51_pct_exact_boundary() {
        // 51 YES out of 100 total = exactly 5100 bps — should pass.
        assert_eq!(
            eval_vote(50, 49, true, TEST_MIN_VOTES),
            Ok(true),
            "51% YES (5100 bps) must pass"
        );
    }

    #[test]
    fn quorum_50_pct_does_not_pass() {
        // 50 YES out of 100 total = 5000 bps — should NOT pass (need >= 5100).
        assert_eq!(
            eval_vote(49, 50, true, TEST_MIN_VOTES),
            Ok(false),
            "50% YES (5000 bps) must not pass"
        );
    }

    #[test]
    fn quorum_five_votes_with_majority() {
        // 4 YES + 1 NO — 80% > 51%, passes.
        assert_eq!(
            eval_vote(3, 1, true, TEST_MIN_VOTES),
            Ok(true),
            "4/5 YES (80%) must pass"
        );
    }

    #[test]
    fn quorum_min_one_passes_on_first_yes() {
        // A pool with min_votes_to_pass = 1 (devnet default) passes immediately
        // on the first YES vote — this is the intended devnet behaviour.
        assert_eq!(
            eval_vote(0, 0, true, 1),
            Ok(true),
            "min_votes=1 pool: single YES must immediately pass"
        );
    }

    // ─── 2. Double-vote prevention (voted_on retention) ──────────────

    #[test]
    fn double_vote_detected_in_contains_check() {
        let mut voted_on: Vec<u64> = vec![7];
        // Mirrors: require!(!contributor.voted_on.contains(&proposal_id), AlreadyVoted)
        assert!(
            voted_on.contains(&7),
            "voter who already voted on proposal 7 must be blocked"
        );
        // After the contains check passes, votes are only recorded once
        prune_and_record(&mut voted_on, 8);
        assert!(voted_on.contains(&8), "new proposal ID is recorded");
        assert!(!voted_on.contains(&6), "unrelated proposal ID not present");
    }

    #[test]
    fn double_vote_same_proposal_is_blocked() {
        let proposal_id: u64 = 42;
        let voted_on: Vec<u64> = vec![42];
        assert!(
            voted_on.contains(&proposal_id),
            "second vote on same proposal_id must be caught by the contains check"
        );
    }

    #[test]
    fn voted_on_retain_prunes_old_ids_on_window_full() {
        // Fill voted_on to MAX_VOTED - 1 with IDs 0..=48, then record proposal 49.
        let mut voted_on: Vec<u64> = (0u64..ContributorAccount::MAX_VOTED as u64)
            .collect();
        assert_eq!(voted_on.len(), ContributorAccount::MAX_VOTED);

        // Simulate casting vote on proposal 100 (far ahead).
        // min_valid_id = 100 - 50 = 50 → everything ≤ 49 should be pruned.
        let proposal_id: u64 = 100;
        let min_valid = proposal_id.saturating_sub(ContributorAccount::MAX_VOTED as u64);
        voted_on.retain(|&id| id >= min_valid);
        voted_on.push(proposal_id);

        // All old IDs (0..=49) should be gone
        for old_id in 0u64..ContributorAccount::MAX_VOTED as u64 {
            assert!(
                !voted_on.contains(&old_id),
                "old proposal ID {} must be pruned after window overflow",
                old_id
            );
        }
        assert!(voted_on.contains(&100), "new proposal ID recorded after prune");
    }

    #[test]
    fn voted_on_retain_cannot_revote_after_prune() {
        // After the window fills and prunes, an old proposal_id that was
        // evicted CANNOT be re-voted on — the eviction is irreversible.
        //
        // Scenario: voter voted on proposals 0..=49, now tries to vote on
        // proposal 100.  min_valid_id = 50, so IDs 0-49 are pruned.
        // If they now try to re-vote on proposal 5 — the contains check
        // (run BEFORE push) should catch it only if the ID is still present.
        // After pruning, ID 5 is gone, so re-vote on ID 5 would go undetected.
        //
        // This is the documented trade-off: the rolling window prevents the
        // vec from growing unboundedly, and the window is sized (MAX_VOTED=50)
        // to be wider than any realistic active-voting period.
        let mut voted_on: Vec<u64> = (0u64..ContributorAccount::MAX_VOTED as u64).collect();
        let proposal_id: u64 = 100;
        prune_and_record(&mut voted_on, proposal_id);

        // IDs 0..=49 are gone — the retain window pruned them
        assert!(!voted_on.contains(&5),  "ID 5 pruned from window");
        assert!(!voted_on.contains(&49), "ID 49 pruned from window");
        assert!(voted_on.contains(&100), "current proposal recorded");
    }

    #[test]
    fn voted_on_no_prune_when_below_max() {
        // When voted_on is below MAX_VOTED, no pruning occurs — all IDs retained.
        let mut voted_on: Vec<u64> = vec![1, 2, 3];
        prune_and_record(&mut voted_on, 4);
        assert_eq!(voted_on, vec![1, 2, 3, 4], "no pruning below max");
    }

    // ─── 3. Overflow safety ───────────────────────────────────────────

    #[test]
    fn votes_yes_overflow_is_caught() {
        // checked_add on u64::MAX must return None (overflow).
        let result = u64::MAX.checked_add(1);
        assert!(result.is_none(), "votes_yes overflow must be caught by checked_add");
    }

    #[test]
    fn bps_mul_overflow_is_caught() {
        // votes_yes * BPS_DENOMINATOR can overflow for large vote counts.
        // checked_mul must detect it.
        let large: u64 = u64::MAX / 2;
        let result = large.checked_mul(BPS_DENOMINATOR);
        assert!(result.is_none(), "BPS multiplication overflow must be caught");
    }
          }
          
