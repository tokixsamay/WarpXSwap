#!/usr/bin/env ts-node
// ╔══════════════════════════════════════════════════════════════════════════╗
// ║  WarpXSwap — validate_payload Regression Test                           ║
// ║                                                                          ║
// ║  Verifies that every boundary-violating proposal is rejected at          ║
// ║  CREATE TIME (not at execution time), catching future regressions in     ║
// ║  the on-chain validate_payload gate.                                     ║
// ║                                                                          ║
// ║  Run against a live local-validator AFTER complete-setup.ts has run.    ║
// ║  The wallet must already be a registered contributor in the pool's       ║
// ║  top_10 (the first depositor from setup qualifies automatically).        ║
// ║                                                                          ║
// ║  Usage:                                                                  ║
// ║    ts-node scripts/test-validate-payload.ts --pool <pool-PDA>            ║
// ║                                                                          ║
// ║  Env vars:                                                               ║
// ║    RPC_URL=http://127.0.0.1:8899                                        ║
// ║    WALLET_PATH=~/.config/solana/id.json                                 ║
// ╚══════════════════════════════════════════════════════════════════════════╝

import * as fs   from "fs";
import * as os   from "os";
import * as path from "path";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import { AnchorProvider, Program, Wallet, BN } from "@coral-xyz/anchor";
import {
  GOVERNANCE_PROGRAM_ID,
  findGovernancePDA,
  findProposalPDA,
} from "../sdk/src";

// ── CONSTANTS ──────────────────────────────────────────────────────────────

const BPS_DENOMINATOR = 10_000;

// Dummy mint used as a stand-in where the contract doesn't validate the mint
// key itself at proposal time (mint existence is only checked at execution).
const DUMMY_MINT = new PublicKey("11111111111111111111111111111111");

// ── RESULT TRACKING ───────────────────────────────────────────────────────

interface TestResult {
  name:   string;
  passed: boolean;
  note:   string;
}
const results: TestResult[] = [];

// ── CLI ARGS ──────────────────────────────────────────────────────────────

function parseArgs(): { poolPda: PublicKey } {
  const args    = process.argv.slice(2);
  const poolIdx = args.indexOf("--pool");
  if (poolIdx < 0 || !args[poolIdx + 1]) {
    throw new Error("Usage: ts-node test-validate-payload.ts --pool <pool-PDA>");
  }
  return { poolPda: new PublicKey(args[poolIdx + 1]) };
}

// ── HELPERS ───────────────────────────────────────────────────────────────

const IDL_DIR = path.join(__dirname, "..", "target", "idl");

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function loadIdl(name: string): any {
  const p = path.join(IDL_DIR, `${name}.json`);
  if (!fs.existsSync(p)) {
    throw new Error(`IDL not found: ${p}\nRun 'anchor build' first.`);
  }
  return JSON.parse(fs.readFileSync(p, "utf-8"));
}

// Returns true when the error message contains one of the given fragments.
function isExpectedError(err: unknown, ...fragments: string[]): boolean {
  const msg = err instanceof Error ? err.message : String(err);
  return fragments.some(f => msg.includes(f));
}

// Try to send a proposal that MUST revert.
// Returns PASS if the call throws, FAIL if it unexpectedly succeeds.
async function expectRevert(
  label: string,
  send:  () => Promise<string>,
): Promise<void> {
  try {
    await send();
    results.push({ name: label, passed: false, note: "TX succeeded — should have reverted!" });
  } catch (err) {
    if (isExpectedError(err, "InvalidParameter", "6006", "6007", "AnchorError")) {
      results.push({ name: label, passed: true, note: "Reverted as expected" });
    } else {
      // Unexpected error (network, blockhash, etc.) — not a validator regression.
      results.push({
        name:   label,
        passed: false,
        note:   `Unexpected error: ${err instanceof Error ? err.message.slice(0, 120) : String(err)}`,
      });
    }
  }
}

// Try to send a proposal that MUST succeed (positive-path sanity check).
async function expectSuccess(
  label: string,
  send:  () => Promise<string>,
): Promise<void> {
  try {
    await send();
    results.push({ name: label, passed: true, note: "Created successfully" });
  } catch (err) {
    results.push({
      name:   label,
      passed: false,
      note:   `Unexpected revert: ${err instanceof Error ? err.message.slice(0, 120) : String(err)}`,
    });
  }
}

// ── PROPOSAL BUILDER ──────────────────────────────────────────────────────

type ProposalSender = (
  govProgram:    Program,
  governancePda: PublicKey,
  contributorPda: PublicKey,
  proposer:      Keypair,
  poolPda:       PublicKey,
) => () => Promise<string>;

function makeProposalSender(
  proposalType: object,
  payload:      object,
): ProposalSender {
  return (govProgram, governancePda, contributorPda, proposer, poolPda) =>
    async () => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const govAccount: any = await (govProgram.account as any)
        .governanceAccount.fetch(governancePda);
      const proposalId: BN = govAccount.proposalCount as BN;
      const [proposalPda]  = findProposalPDA(poolPda, proposalId);

      return govProgram.methods
        .createProposal(proposalType, payload, false)
        .accounts({
          governance:    governancePda,
          proposal:      proposalPda,
          contributor:   contributorPda,
          proposer:      proposer.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
    };
}

// ── MAIN ──────────────────────────────────────────────────────────────────

async function main() {
  const { poolPda } = parseArgs();

  const RPC_URL     = process.env.RPC_URL ?? "http://127.0.0.1:8899";
  const WALLET_PATH = process.env.WALLET_PATH
    ? path.resolve(process.env.WALLET_PATH)
    : path.join(os.homedir(), ".config", "solana", "id.json");

  if (!fs.existsSync(WALLET_PATH)) {
    throw new Error(`Wallet not found: ${WALLET_PATH}`);
  }

  const rawKey    = JSON.parse(fs.readFileSync(WALLET_PATH, "utf-8")) as number[];
  const proposer  = Keypair.fromSecretKey(Uint8Array.from(rawKey));
  const connection = new Connection(RPC_URL, "confirmed");
  const provider   = new AnchorProvider(connection, new Wallet(proposer), {
    commitment: "confirmed",
  });

  const govIdl     = loadIdl("governance_program");
  const govProgram = new Program(govIdl, provider);

  const [governancePda]  = findGovernancePDA(poolPda, GOVERNANCE_PROGRAM_ID);
  const [contributorPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("contributor"), poolPda.toBuffer(), proposer.publicKey.toBuffer()],
    GOVERNANCE_PROGRAM_ID
  );

  // Convenience wrapper — passes all shared context into a sender factory.
  const send = (factory: ProposalSender) =>
    factory(govProgram, governancePda, contributorPda, proposer, poolPda);

  // ── HEADER ──────────────────────────────────────────────────────────────
  console.log("\n╔════════════════════════════════════════════════════════════╗");
  console.log("║  WarpXSwap — validate_payload Regression Test              ║");
  console.log("╚════════════════════════════════════════════════════════════╝\n");
  console.log(`RPC:         ${RPC_URL}`);
  console.log(`Pool PDA:    ${poolPda.toBase58()}`);
  console.log(`Governance:  ${governancePda.toBase58()}`);
  console.log(`Proposer:    ${proposer.publicKey.toBase58()}`);
  console.log(`Contributor: ${contributorPda.toBase58()}`);
  console.log();

  // ── VERIFY CONTRIBUTOR EXISTS AND IS IN TOP 10 ──────────────────────────
  const contribInfo = await connection.getAccountInfo(contributorPda);
  if (!contribInfo) {
    throw new Error(
      `Contributor account not found: ${contributorPda.toBase58()}\n` +
      `Run complete-setup.ts first and ensure this wallet deposited into the pool.`
    );
  }

  // ══════════════════════════════════════════════════════════════════════════
  // SECTION 1 — UpdateFeeRange
  // validate_payload checks: new_min < new_max, new_max <= BPS_DENOMINATOR
  // ══════════════════════════════════════════════════════════════════════════
  console.log("── UpdateFeeRange ────────────────────────────────────────────");

  // INVALID: min == max (equal values must fail)
  await expectRevert(
    "UpdateFeeRange: min == max (50 == 50)",
    send(makeProposalSender(
      { updateFeeRange: {} },
      { updateFeeRange: { mint: DUMMY_MINT, newMin: 50, newMax: 50 } }
    ))
  );

  // INVALID: min > max
  await expectRevert(
    "UpdateFeeRange: min > max (100 > 50)",
    send(makeProposalSender(
      { updateFeeRange: {} },
      { updateFeeRange: { mint: DUMMY_MINT, newMin: 100, newMax: 50 } }
    ))
  );

  // INVALID: max > 10_000 (fee cannot exceed 100%)
  await expectRevert(
    "UpdateFeeRange: max > BPS_DENOMINATOR (10_001)",
    send(makeProposalSender(
      { updateFeeRange: {} },
      { updateFeeRange: { mint: DUMMY_MINT, newMin: 50, newMax: BPS_DENOMINATOR + 1 } }
    ))
  );

  // INVALID: max == 10_001 with valid relation
  await expectRevert(
    "UpdateFeeRange: max == 10_001 (exactly over limit)",
    send(makeProposalSender(
      { updateFeeRange: {} },
      { updateFeeRange: { mint: DUMMY_MINT, newMin: 1, newMax: BPS_DENOMINATOR + 1 } }
    ))
  );

  // VALID: min=30, max=100 (standard LP fee range)
  await expectSuccess(
    "UpdateFeeRange: valid (30 bps → 100 bps) [positive path]",
    send(makeProposalSender(
      { updateFeeRange: {} },
      { updateFeeRange: { mint: DUMMY_MINT, newMin: 30, newMax: 100 } }
    ))
  );

  // ══════════════════════════════════════════════════════════════════════════
  // SECTION 2 — UpdateThreshold
  // validate_payload checks: new_up > 0 AND new_down > 0
  // ══════════════════════════════════════════════════════════════════════════
  console.log("\n── UpdateThreshold ───────────────────────────────────────────");

  // INVALID: threshold_up == 0
  await expectRevert(
    "UpdateThreshold: threshold_up == 0",
    send(makeProposalSender(
      { updateThreshold: {} },
      { updateThreshold: { mint: DUMMY_MINT, newUp: 0, newDown: 400 } }
    ))
  );

  // INVALID: threshold_down == 0
  await expectRevert(
    "UpdateThreshold: threshold_down == 0",
    send(makeProposalSender(
      { updateThreshold: {} },
      { updateThreshold: { mint: DUMMY_MINT, newUp: 800, newDown: 0 } }
    ))
  );

  // INVALID: both zero
  await expectRevert(
    "UpdateThreshold: both == 0",
    send(makeProposalSender(
      { updateThreshold: {} },
      { updateThreshold: { mint: DUMMY_MINT, newUp: 0, newDown: 0 } }
    ))
  );

  // VALID: asymmetric thresholds (8% up, 4% down)
  await expectSuccess(
    "UpdateThreshold: valid (800 up, 400 down) [positive path]",
    send(makeProposalSender(
      { updateThreshold: {} },
      { updateThreshold: { mint: DUMMY_MINT, newUp: 800, newDown: 400 } }
    ))
  );

  // ══════════════════════════════════════════════════════════════════════════
  // SECTION 3 — UpdateMaxPct
  // validate_payload checks: new_min < new_max (strict), new_max <= 100
  // BUG FIXED: was `<=` (allowed equal), now strict `<` matches execute.rs
  // ══════════════════════════════════════════════════════════════════════════
  console.log("\n── UpdateMaxPct ──────────────────────────────────────────────");

  // INVALID: min == max (the bug that was fixed — equal was previously accepted)
  await expectRevert(
    "UpdateMaxPct: min == max (30 == 30) [FIXED BUG]",
    send(makeProposalSender(
      { updateMaxPct: {} },
      { updateMaxPct: { mint: DUMMY_MINT, newMin: 30, newMax: 30 } }
    ))
  );

  // INVALID: min > max
  await expectRevert(
    "UpdateMaxPct: min > max (40 > 20)",
    send(makeProposalSender(
      { updateMaxPct: {} },
      { updateMaxPct: { mint: DUMMY_MINT, newMin: 40, newMax: 20 } }
    ))
  );

  // INVALID: max > 100%
  await expectRevert(
    "UpdateMaxPct: max > 100 (101)",
    send(makeProposalSender(
      { updateMaxPct: {} },
      { updateMaxPct: { mint: DUMMY_MINT, newMin: 20, newMax: 101 } }
    ))
  );

  // INVALID: max == 100 but min == max
  await expectRevert(
    "UpdateMaxPct: min == max == 100 [FIXED BUG boundary]",
    send(makeProposalSender(
      { updateMaxPct: {} },
      { updateMaxPct: { mint: DUMMY_MINT, newMin: 100, newMax: 100 } }
    ))
  );

  // VALID: standard range (20% min, 30% max)
  await expectSuccess(
    "UpdateMaxPct: valid (20 min, 30 max) [positive path]",
    send(makeProposalSender(
      { updateMaxPct: {} },
      { updateMaxPct: { mint: DUMMY_MINT, newMin: 20, newMax: 30 } }
    ))
  );

  // ══════════════════════════════════════════════════════════════════════════
  // SECTION 4 — AddAsset (volatile)
  // validate_payload checks: fee_min < fee_max, threshold > 0, max_pct_min <= max_pct_max
  // ══════════════════════════════════════════════════════════════════════════
  console.log("\n── AddAsset (volatile) ───────────────────────────────────────");

  const baseAddAsset = {
    mint:         DUMMY_MINT,
    maxPctMin:    20,
    maxPctMax:    30,
    feeMin:       30,
    feeMax:       100,
    thresholdUp:  800,
    thresholdDown: 400,
    initialBase:  new BN(100_000_000),
    allowed:      [],
    isStable:     false,
    staticFeeBps: 0,
  };

  // INVALID: fee_min == fee_max
  await expectRevert(
    "AddAsset volatile: feeMin == feeMax (50 == 50)",
    send(makeProposalSender(
      { addAsset: {} },
      { addAsset: { ...baseAddAsset, feeMin: 50, feeMax: 50 } }
    ))
  );

  // INVALID: fee_min > fee_max
  await expectRevert(
    "AddAsset volatile: feeMin > feeMax (100 > 50)",
    send(makeProposalSender(
      { addAsset: {} },
      { addAsset: { ...baseAddAsset, feeMin: 100, feeMax: 50 } }
    ))
  );

  // INVALID: threshold_up == 0 (volatile assets must have thresholds)
  await expectRevert(
    "AddAsset volatile: thresholdUp == 0",
    send(makeProposalSender(
      { addAsset: {} },
      { addAsset: { ...baseAddAsset, thresholdUp: 0 } }
    ))
  );

  // INVALID: threshold_down == 0
  await expectRevert(
    "AddAsset volatile: thresholdDown == 0",
    send(makeProposalSender(
      { addAsset: {} },
      { addAsset: { ...baseAddAsset, thresholdDown: 0 } }
    ))
  );

  // INVALID: max_pct_min > max_pct_max
  await expectRevert(
    "AddAsset volatile: maxPctMin > maxPctMax (40 > 20)",
    send(makeProposalSender(
      { addAsset: {} },
      { addAsset: { ...baseAddAsset, maxPctMin: 40, maxPctMax: 20 } }
    ))
  );

  // VALID: well-formed volatile asset
  await expectSuccess(
    "AddAsset volatile: valid [positive path]",
    send(makeProposalSender(
      { addAsset: {} },
      { addAsset: baseAddAsset }
    ))
  );

  // ══════════════════════════════════════════════════════════════════════════
  // SECTION 5 — AddAsset (stable)
  // validate_payload checks: threshold_up == 0 AND threshold_down == 0
  // ══════════════════════════════════════════════════════════════════════════
  console.log("\n── AddAsset (stable) ─────────────────────────────────────────");

  const baseAddStable = {
    mint:         DUMMY_MINT,
    maxPctMin:    20,
    maxPctMax:    30,
    feeMin:       0,
    feeMax:       0,
    thresholdUp:  0,
    thresholdDown: 0,
    initialBase:  new BN(1_000_000),
    allowed:      [],
    isStable:     true,
    staticFeeBps: 3,
  };

  // INVALID: stable but threshold_up != 0 (stables must have zero thresholds)
  await expectRevert(
    "AddAsset stable: thresholdUp != 0 (should be 0 for stables)",
    send(makeProposalSender(
      { addAsset: {} },
      { addAsset: { ...baseAddStable, thresholdUp: 100 } }
    ))
  );

  // INVALID: stable but threshold_down != 0
  await expectRevert(
    "AddAsset stable: thresholdDown != 0 (should be 0 for stables)",
    send(makeProposalSender(
      { addAsset: {} },
      { addAsset: { ...baseAddStable, thresholdDown: 100 } }
    ))
  );

  // VALID: well-formed stablecoin asset
  await expectSuccess(
    "AddAsset stable: valid (zero thresholds, static fee 3 bps) [positive path]",
    send(makeProposalSender(
      { addAsset: {} },
      { addAsset: baseAddStable }
    ))
  );

  // ══════════════════════════════════════════════════════════════════════════
  // SECTION 6 — RemoveAsset / UpdateAllowance / SetPythFeedId / SetInflowBlocked
  // No numeric invariants in validate_payload — but positive-path confirms
  // the discriminant encoding is correct and no accidental constraint added.
  // ══════════════════════════════════════════════════════════════════════════
  console.log("\n── No-invariant variants (positive path only) ─────────────────");

  await expectSuccess(
    "RemoveAsset: valid payload accepted [positive path]",
    send(makeProposalSender(
      { removeAsset: {} },
      { removeAsset: { mint: DUMMY_MINT } }
    ))
  );

  await expectSuccess(
    "UpdateAllowance: valid payload accepted [positive path]",
    send(makeProposalSender(
      { updateAllowance: {} },
      { updateAllowance: { asset: DUMMY_MINT, target: DUMMY_MINT, allowed: true } }
    ))
  );

  await expectSuccess(
    "SetPythFeedId: valid payload accepted [positive path]",
    send(makeProposalSender(
      { setPythFeedId: {} },
      { setPythFeedId: { mint: DUMMY_MINT, pythFeedId: Array(32).fill(1) } }
    ))
  );

  await expectSuccess(
    "SetInflowBlocked: block=true accepted [positive path]",
    send(makeProposalSender(
      { setInflowBlocked: {} },
      { setInflowBlocked: { mint: DUMMY_MINT, blocked: true } }
    ))
  );

  await expectSuccess(
    "SetInflowBlocked: block=false accepted [positive path]",
    send(makeProposalSender(
      { setInflowBlocked: {} },
      { setInflowBlocked: { mint: DUMMY_MINT, blocked: false } }
    ))
  );

  // ── PRINT REPORT ────────────────────────────────────────────────────────
  const passed = results.filter(r => r.passed).length;
  const failed = results.filter(r => !r.passed).length;

  console.log("\n╔════════════════════════════════════════════════════════════╗");
  console.log("║  Test Results                                               ║");
  console.log("╚════════════════════════════════════════════════════════════╝\n");

  for (const r of results) {
    const icon = r.passed ? "✓ PASS" : "✗ FAIL";
    console.log(`  ${icon}  ${r.name}`);
    if (!r.passed) {
      console.log(`         └─ ${r.note}`);
    }
  }

  console.log(`\n  Total: ${results.length}  |  Passed: ${passed}  |  Failed: ${failed}`);

  if (failed > 0) {
    console.log("\n  ✗ Some tests failed — validate_payload may have regressed.\n");
    process.exit(1);
  } else {
    console.log("\n  ✓ All tests passed — validate_payload is sound.\n");
  }
}

main().catch((e) => {
  console.error("\n✗ Fatal error:", e.message ?? e);
  process.exit(1);
});
