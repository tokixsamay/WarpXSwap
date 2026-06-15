#!/usr/bin/env ts-node
// ╔═══════════════════════════════════════════════════════════════════╗
// ║   WarpXSwap — Governance Crank                                   ║
// ║                                                                   ║
// ║   Polls all known pools for Passed (un-executed) proposals       ║
// ║   and executes them automatically via execute_proposal.          ║
// ║                                                                   ║
// ║   Handles all proposal types:                                     ║
// ║     • UpdateFeeRange      • UpdateThreshold                       ║
// ║     • UpdateMaxPct        • UpdateAllowance                       ║
// ║     • AddAsset            • RemoveAsset                           ║
// ║     • SetPythFeedId       (new — Pyth V2 feed rotation)           ║
// ║                                                                   ║
// ║   Also logs Active proposals that have expired without passing   ║
// ║   (expired-without-quorum detection, on-chain status unchanged). ║
// ╚═══════════════════════════════════════════════════════════════════╝
//
// Usage:
//   ts-node scripts/govern-crank.ts                          normal crank (polls + executes)
//   ts-node scripts/govern-crank.ts --quorum-check           print live vote tallies and exit
//   ts-node scripts/govern-crank.ts --quorum-check --watch   live-refresh tally every N seconds
//   ts-node scripts/govern-crank.ts --finalize-expired       finalize all expired proposals and exit
//
// Env vars (all optional — defaults shown):
//   RPC_URL=http://127.0.0.1:8899
//   WALLET_PATH=~/.config/solana/id.json    (executor keypair — pays tx fees)
//   POOLS=<base58,base58,...>               comma-separated pool PDAs to watch
//                                           (if unset, reads from setup-output.json)
//   GOVERN_INTERVAL_MS=30000               poll interval (default 30 s)
//   WATCH_INTERVAL_MS=30000                refresh interval for --quorum-check --watch (default 30 s)
//   LOG_LEVEL=info                         info | debug | quiet
//   DRY_RUN=false                          true = detect but do not execute

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
  POOL_PROGRAM_ID,
  INFO_POOL_PROGRAM_ID,
  findGovernancePDA,
  findInfoPoolPDA,
  findAssetPDA,
} from "../sdk/src";

// ── CONFIG ────────────────────────────────────────────────────────────

const QUORUM_CHECK     = process.argv.includes("--quorum-check");
const WATCH_MODE       = process.argv.includes("--watch");
const FINALIZE_EXPIRED = process.argv.includes("--finalize-expired");
const RPC_URL          = process.env.RPC_URL ?? "http://127.0.0.1:8899";
const GOVERN_MS        = parseInt(process.env.GOVERN_INTERVAL_MS ?? "30000", 10);
const WATCH_MS         = parseInt(process.env.WATCH_INTERVAL_MS  ?? "30000", 10);
const DRY_RUN      = (process.env.DRY_RUN ?? "false") === "true";
const LOG_LEVEL    = (process.env.LOG_LEVEL ?? "info") as "info" | "debug" | "quiet";

const WALLET_PATH  = process.env.WALLET_PATH
  ? path.resolve(process.env.WALLET_PATH)
  : path.join(os.homedir(), ".config", "solana", "id.json");

const IDL_DIR = path.join(__dirname, "..", "target", "idl");

// ── LOGGING ───────────────────────────────────────────────────────────

const ts = () => new Date().toISOString();

function logInfo(msg: string)  { if (LOG_LEVEL !== "quiet") console.log(`[${ts()}] ${msg}`); }
function logDebug(msg: string) { if (LOG_LEVEL === "debug") console.log(`[${ts()}] [debug] ${msg}`); }
function logWarn(msg: string)  { console.warn(`[${ts()}] [warn]  ${msg}`); }
function logError(msg: string) { console.error(`[${ts()}] [ERROR] ${msg}`); }

// ── IDL LOADER ────────────────────────────────────────────────────────

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function loadIdl(name: string): any {
  const p = path.join(IDL_DIR, `${name}.json`);
  if (!fs.existsSync(p)) {
    throw new Error(`IDL not found: ${p}\nRun 'anchor build' first.`);
  }
  return JSON.parse(fs.readFileSync(p, "utf-8"));
}

// ── POOL DISCOVERY ────────────────────────────────────────────────────

function discoverPools(): PublicKey[] {
  // 1. Explicit env override
  if (process.env.POOLS) {
    const addrs = process.env.POOLS.split(",")
      .map(s => s.trim())
      .filter(Boolean);
    if (addrs.length > 0) {
      logInfo(`Watching ${addrs.length} pool(s) from POOLS env`);
      return addrs.map(a => new PublicKey(a));
    }
  }

  // 2. complete-setup.json written by complete-setup.ts (one dir above scripts/)
  const outputPath = path.join(__dirname, "..", "complete-setup.json");
  if (fs.existsSync(outputPath)) {
    try {
      const data = JSON.parse(fs.readFileSync(outputPath, "utf-8")) as Record<
        string,
        { poolPda?: string }
      >;
      // Extract poolPda from every top-level pool object
      const pdaStrs = Object.values(data)
        .map(v => v?.poolPda)
        .filter((s): s is string => typeof s === "string" && s.length > 0);
      if (pdaStrs.length > 0) {
        logInfo(`Watching ${pdaStrs.length} pool(s) from complete-setup.json`);
        return pdaStrs.map(s => new PublicKey(s));
      }
    } catch {
      logWarn("Could not parse complete-setup.json — falling back to empty pool list");
    }
  }

  logWarn(
    "No pools configured. Set POOLS=<pda1,pda2,...> or generate setup-output.json.\n" +
    "         Crank will idle until pools are configured."
  );
  return [];
}

// ── PAYLOAD HELPERS ───────────────────────────────────────────────────

// Extract the primary mint pubkey from any payload variant.
// For UpdateAllowance the "asset" field is the mint whose allowance list is updated.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function extractMintFromPayload(payload: any): PublicKey | null {
  try {
    if (payload.updateFeeRange)    return new PublicKey(payload.updateFeeRange.mint);
    if (payload.updateThreshold)   return new PublicKey(payload.updateThreshold.mint);
    if (payload.updateMaxPct)      return new PublicKey(payload.updateMaxPct.mint);
    if (payload.addAsset)          return new PublicKey(payload.addAsset.mint);
    if (payload.removeAsset)       return new PublicKey(payload.removeAsset.mint);
    if (payload.updateAllowance)   return new PublicKey(payload.updateAllowance.asset);
    if (payload.setPythFeedId)     return new PublicKey(payload.setPythFeedId.mint);
    if (payload.setInflowBlocked)  return new PublicKey(payload.setInflowBlocked.mint);
  } catch { }
  return null;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function payloadTypeName(payload: any): string {
  if (payload.updateFeeRange)   return "UpdateFeeRange";
  if (payload.updateThreshold)  return "UpdateThreshold";
  if (payload.updateMaxPct)     return "UpdateMaxPct";
  if (payload.addAsset)         return "AddAsset";
  if (payload.removeAsset)      return "RemoveAsset";
  if (payload.updateAllowance)  return "UpdateAllowance";
  if (payload.setPythFeedId)    return "SetPythFeedId";
  if (payload.setInflowBlocked) return "SetInflowBlocked";
  return "Unknown";
}

// ── PROPOSAL EXECUTOR ─────────────────────────────────────────────────

interface ExecContext {
  govProgram:  Program;
  poolKey:     PublicKey;          // pool PDA (= governance.pool_id)
  govPda:      PublicKey;
  executor:    Keypair;
  govTop10:    PublicKey[];        // snapshot of governance.top_10 for this pool
}

async function executeProposal(
  ctx: ExecContext,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  proposal: any,
  proposalId: BN,
  proposalPda: PublicKey
): Promise<void> {
  const { govProgram, poolKey, govPda, executor } = ctx;

  const mint = extractMintFromPayload(proposal.payload);
  if (!mint) {
    logWarn(
      `Proposal #${proposalId} — cannot extract mint from payload ` +
      `(type=${payloadTypeName(proposal.payload)}), skipping`
    );
    return;
  }

  // Derive the asset PDA for this mint
  const [assetPda]    = findAssetPDA(poolKey, mint, POOL_PROGRAM_ID);
  const [infoPoolPda] = findInfoPoolPDA(poolKey, INFO_POOL_PROGRAM_ID);

  logInfo(
    `Executing proposal #${proposalId} [${payloadTypeName(proposal.payload)}] ` +
    `mint=${mint.toBase58().slice(0, 8)}…`
  );
  logDebug(`  poolKey:     ${poolKey.toBase58()}`);
  logDebug(`  assetPda:    ${assetPda.toBase58()}`);
  logDebug(`  infoPoolPda: ${infoPoolPda.toBase58()}`);

  if (DRY_RUN) {
    logInfo(`  [DRY RUN] Would execute proposal #${proposalId} — skipped.`);
    return;
  }

  const tx = await govProgram.methods
    .executeProposal(proposalId)
    .accounts({
      governance:       govPda,
      proposal:         proposalPda,
      poolProgram:      POOL_PROGRAM_ID,
      infoPoolProgram:  INFO_POOL_PROGRAM_ID,
      poolAccount:      poolKey,
      assetAccount:     assetPda,
      infoPoolAccount:  infoPoolPda,
      executor:         executor.publicKey,
      systemProgram:    SystemProgram.programId,
    })
    .rpc();

  logInfo(`  ✓ Executed! tx=${tx}`);
}

// ── EMERGENCY PROPOSAL HANDLER ────────────────────────────────────────

async function handleEmergencyProposal(
  ctx: ExecContext,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  proposal: any,
  proposalId: BN,
  proposalPda: PublicKey,
): Promise<void> {
  const { govProgram, govPda, executor, govTop10 } = ctx;

  const alreadyApproved: PublicKey[] = (proposal.emergencyApprovals as PublicKey[]) ?? [];
  const alreadyApprovedSet = new Set(alreadyApproved.map(k => k.toBase58()));

  const pendingApprovers = govTop10.filter(k => !alreadyApprovedSet.has(k.toBase58()));
  const needed  = Math.floor(govTop10.length / 2) + 1;   // majority_needed = (n/2)+1
  const current = alreadyApproved.length;
  const remaining = needed - current;

  logInfo(
    `  Proposal #${proposalId} [${payloadTypeName(proposal.payload)}] EMERGENCY — ` +
    `${current}/${needed} approvals (need ${remaining} more)`
  );

  // Log who still needs to sign
  if (pendingApprovers.length > 0) {
    logInfo(`  Pending Top-10 approvers (${pendingApprovers.length}):`);
    for (const pk of pendingApprovers) {
      logInfo(`    • ${pk.toBase58()}`);
    }
  }

  // Auto-approve if the executor's own key is in top_10 and hasn't approved yet
  const executorKey = executor.publicKey.toBase58();
  const executorInTop10  = govTop10.some(k => k.toBase58() === executorKey);
  const executorApproved = alreadyApprovedSet.has(executorKey);

  if (!executorInTop10) {
    logDebug(`  Executor not in Top-10 — cannot auto-approve this emergency proposal`);
    return;
  }

  if (executorApproved) {
    logDebug(`  Executor already approved proposal #${proposalId}`);
    return;
  }

  logInfo(`  Executor IS in Top-10 and has not yet approved — auto-approving…`);

  if (DRY_RUN) {
    logInfo(`  [DRY RUN] Would call approve_emergency #${proposalId} — skipped.`);
    return;
  }

  try {
    const tx = await govProgram.methods
      .approveEmergency(proposalId)
      .accounts({
        governance: govPda,
        proposal:   proposalPda,
        approver:   executor.publicKey,
      })
      .rpc();

    const newCount = current + 1;
    if (newCount >= needed) {
      logInfo(`  ✓ Approved! Majority reached (${newCount}/${needed}) — proposal now Active. tx=${tx}`);
    } else {
      logInfo(`  ✓ Approved! (${newCount}/${needed}). tx=${tx}`);
    }
  } catch (e) {
    logError(`  approve_emergency failed: ${(e as Error).message ?? e}`);
  }
}

// ── POLL ONE POOL ─────────────────────────────────────────────────────

async function pollPool(
  poolKey: PublicKey,
  govProgram: Program,
  executor: Keypair,
  now: number
): Promise<{ checked: number; executed: number; expired: number; emergency: number }> {
  const stats = { checked: 0, executed: 0, expired: 0, emergency: 0 };

  const [govPda] = findGovernancePDA(poolKey, GOVERNANCE_PROGRAM_ID);

  let govAccount: { proposalCount: BN; poolId: PublicKey; top10: PublicKey[] };
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    govAccount = await (govProgram.account as any).governanceAccount.fetch(govPda) as any;
  } catch (e) {
    logDebug(`Pool ${poolKey.toBase58().slice(0, 8)}… — governance PDA not found, skip`);
    return stats;
  }

  const count    = govAccount.proposalCount.toNumber();
  const govTop10 = govAccount.top10 ?? [];
  logDebug(`Pool ${poolKey.toBase58().slice(0, 8)}… — ${count} proposal(s), top10=${govTop10.length}`);

  const ctx: ExecContext = { govProgram, poolKey, govPda, executor, govTop10 };

  for (let i = 0; i < count; i++) {
    const proposalId = new BN(i);
    const [proposalPda] = PublicKey.findProgramAddressSync(
      [
        Buffer.from("proposal"),
        poolKey.toBuffer(),
        proposalId.toArrayLike(Buffer, "le", 8),
      ],
      GOVERNANCE_PROGRAM_ID
    );

    let proposal: {
      status:               { active?: object; passed?: object; executed?: object; rejected?: object; pendingEmergencyApproval?: object };
      executed:             boolean;
      endsAt:               BN;
      votesYes:             BN;
      votesNo:              BN;
      payload:              object;
      proposalId:           BN;
      isEmergency:          boolean;
      emergencyApprovals:   PublicKey[];
    };
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      proposal = await (govProgram.account as any).proposalAccount.fetch(proposalPda) as any;
    } catch {
      logDebug(`  Proposal #${i}: PDA not found, skipping`);
      continue;
    }

    stats.checked++;

    // Anchor camelCases enum variants: "pendingEmergencyApproval" | "active" | "passed" | "rejected" | "executed"
    const statusKey = Object.keys(proposal.status)[0];
    logDebug(
      `  Proposal #${i}: status=${statusKey} executed=${proposal.executed} ` +
      `emergency=${proposal.isEmergency} ` +
      `endsAt=${proposal.endsAt.toNumber() > 0 ? new Date(proposal.endsAt.toNumber() * 1000).toISOString() : "not-set"}`
    );

    // ── Already executed ──────────────────────────────────────────
    if (statusKey === "executed" || proposal.executed) {
      logDebug(`  Proposal #${i}: already executed, skip`);
      continue;
    }

    // ── Passed — execute it ───────────────────────────────────────
    if (statusKey === "passed") {
      try {
        await executeProposal(ctx, proposal, proposalId, proposalPda);
        stats.executed++;
      } catch (e) {
        logError(
          `  Proposal #${i} execution failed: ${(e as Error).message ?? e}`
        );
      }
      continue;
    }

    // ── PendingEmergencyApproval — log + auto-approve if executor in Top-10 ──
    if (statusKey === "pendingEmergencyApproval") {
      try {
        await handleEmergencyProposal(ctx, proposal, proposalId, proposalPda);
      } catch (e) {
        logError(`  Proposal #${i} emergency handling failed: ${(e as Error).message ?? e}`);
      }
      stats.emergency++;
      continue;
    }

    // ── Active but past voting window → finalize (mark Rejected) ────
    if (statusKey === "active") {
      const endsAt = proposal.endsAt.toNumber();
      if (endsAt < now) {
        const yes    = proposal.votesYes.toNumber();
        const no     = proposal.votesNo.toNumber();
        const total  = yes + no;
        const yesBps = total > 0 ? Math.floor((yes * 10_000) / total) : 0;

        logInfo(
          `  Proposal #${i} [${payloadTypeName(proposal.payload)}]: ` +
          `expired — ${yes} YES / ${no} NO (${(yesBps / 100).toFixed(1)}%) — ` +
          `finalizing as Rejected…`
        );

        if (DRY_RUN) {
          logInfo(`  [DRY RUN] Would finalize proposal #${i} — skipped.`);
          stats.expired++;
          continue;
        }

        try {
          const tx = await govProgram.methods
            .finalizeProposal(proposalId)
            .accounts({
              governance: govPda,
              proposal:   proposalPda,
              caller:     executor.publicKey,
            })
            .rpc();
          logInfo(`  ✓ Finalized (Rejected). tx=${tx}`);
        } catch (e) {
          logError(
            `  Proposal #${i} finalization failed: ${(e as Error).message ?? e}`
          );
        }
        stats.expired++;
      }
    }
  }

  return stats;
}

// ── QUORUM CHECK ──────────────────────────────────────────────────────

/** Format a seconds-remaining value as "Xd Yh Zm" or "EXPIRED". */
function fmtTimeLeft(secsLeft: number): string {
  if (secsLeft <= 0) return "EXPIRED";
  const d = Math.floor(secsLeft / 86400);
  const h = Math.floor((secsLeft % 86400) / 3600);
  const m = Math.floor((secsLeft % 3600) / 60);
  const parts: string[] = [];
  if (d > 0) parts.push(`${d}d`);
  if (h > 0) parts.push(`${h}h`);
  parts.push(`${m}m`);
  return parts.join(" ");
}

/**
 * YES votes needed to cross the 51% quorum threshold, assuming all
 * remaining votes are YES (i.e. best-case scenario for the proposal).
 *
 * Derivation: need (yes+N)/(yes+no+N) >= 0.51
 *   → 4900·N ≥ 5100·no − 4900·yes
 *   → N = max(0, ⌈(5100·no − 4900·yes) / 4900⌉)
 */
function yesVotesNeededToPass(yes: number, no: number): number {
  const numerator = 5100 * no - 4900 * yes;
  if (numerator <= 0) return 0;                    // already passing
  return Math.ceil(numerator / 4900);
}

/**
 * Prints a live vote tally for every Active proposal across all pools.
 * Reads from the chain but sends no transactions.
 */
async function printQuorumReport(
  pools:      PublicKey[],
  govProgram: Program
): Promise<void> {
  const now = Math.floor(Date.now() / 1000);

  console.log("\n╔═══════════════════════════════════════════════════════╗");
  console.log("║   WarpXSwap — Governance Quorum Check                 ║");
  console.log("╚═══════════════════════════════════════════════════════╝\n");
  console.log(`  RPC:     ${RPC_URL}`);
  console.log(`  Pools:   ${pools.length}`);
  console.log(`  Time:    ${new Date().toISOString()}\n`);

  const QUORUM_BPS = 5100;     // >51 %  (matches governance/constants.rs)
  const BAR_WIDTH  = 20;       // characters for the ASCII progress bar

  let totalActive = 0;

  for (const poolKey of pools) {
    const poolShort = poolKey.toBase58().slice(0, 8) + "…";
    const [govPda]  = findGovernancePDA(poolKey, GOVERNANCE_PROGRAM_ID);

    let govAccount: { proposalCount: BN; poolId: PublicKey; top10: PublicKey[] };
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      govAccount = await (govProgram.account as any).governanceAccount.fetch(govPda) as any;
    } catch {
      console.log(`  Pool ${poolShort}  — governance PDA not found, skipping\n`);
      continue;
    }

    const count    = govAccount.proposalCount.toNumber();
    const activeRows: string[] = [];

    for (let i = 0; i < count; i++) {
      const proposalId = new BN(i);
      const [proposalPda] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("proposal"),
          poolKey.toBuffer(),
          proposalId.toArrayLike(Buffer, "le", 8),
        ],
        GOVERNANCE_PROGRAM_ID
      );

      let proposal: {
        status:             { active?: object; passed?: object; executed?: object; rejected?: object; pendingEmergencyApproval?: object };
        endsAt:             BN;
        votesYes:           BN;
        votesNo:            BN;
        payload:            object;
        isEmergency:        boolean;
        emergencyApprovals: PublicKey[];
      };
      try {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        proposal = await (govProgram.account as any).proposalAccount.fetch(proposalPda) as any;
      } catch {
        continue;
      }

      const statusKey = Object.keys(proposal.status)[0];
      if (statusKey !== "active") continue;   // only Active proposals matter here

      totalActive++;

      const yes      = proposal.votesYes.toNumber();
      const no       = proposal.votesNo.toNumber();
      const total    = yes + no;
      const yesBps   = total > 0 ? Math.floor((yes * 10_000) / total) : 0;
      const yesPct   = (yesBps / 100).toFixed(1);
      const endsAt   = proposal.endsAt.toNumber();
      const secsLeft = endsAt - now;
      const timeLeft = fmtTimeLeft(secsLeft);

      // ASCII bar: filled portion = yes%, empty = no%, remainder = unvoted
      const filledChars = total > 0 ? Math.round((yes / total) * BAR_WIDTH) : 0;
      const emptyChars  = total > 0 ? Math.round((no  / total) * BAR_WIDTH) : 0;
      const restChars   = BAR_WIDTH - filledChars - emptyChars;
      const bar         = "█".repeat(filledChars) + "░".repeat(Math.max(0, emptyChars)) + "·".repeat(Math.max(0, restChars));

      // Quorum status
      const passing       = yesBps >= QUORUM_BPS;
      const yesNeeded     = yesVotesNeededToPass(yes, no);
      const quorumStatus  = passing
        ? "✓ PASSING"
        : yesNeeded === 1 ? "1 more YES to pass"
        : `${yesNeeded} more YES to pass`;

      // Emergency indicator
      const emergencyTag = proposal.isEmergency
        ? ` [EMERGENCY ${proposal.emergencyApprovals.length}/${Math.floor(govAccount.top10.length / 2) + 1} approvals]`
        : "";

      activeRows.push(
        `  #${String(i).padEnd(3)} ` +
        `${payloadTypeName(proposal.payload).padEnd(16)} ` +
        `[${bar}] ` +
        `YES ${yes} / NO ${no}  ${String(yesPct + "%").padStart(6)}  ` +
        `${(passing ? "✓" : "✗")} ${quorumStatus.padEnd(22)}` +
        `⏱ ${timeLeft}` +
        emergencyTag
      );
    }

    if (activeRows.length === 0) {
      console.log(`  Pool ${poolShort}  — no Active proposals`);
    } else {
      console.log(`  Pool ${poolShort}  — ${activeRows.length} Active proposal(s)`);
      console.log(`  ${"─".repeat(110)}`);
      console.log(
        `  ${"#".padEnd(4)} ` +
        `${"Type".padEnd(16)} ` +
        `${"Vote bar".padEnd(BAR_WIDTH + 2)} ` +
        `${"Tally".padEnd(22)} ` +
        `${"Quorum".padEnd(25)} ` +
        `Time left`
      );
      console.log(`  ${"─".repeat(110)}`);
      for (const row of activeRows) {
        console.log(row);
      }
      console.log(`  ${"─".repeat(110)}`);
    }
    console.log();
  }

  console.log(
    totalActive === 0
      ? "  No Active proposals found across any pool.\n"
      : `  Total Active proposals: ${totalActive}\n`
  );
}

// ── FINALIZE EXPIRED ──────────────────────────────────────────────────

interface FinalizeResult {
  pool:       string;
  proposalId: number;
  type:       string;
  yes:        number;
  no:         number;
  yesPct:     string;
  tx:         string | null;   // null = dry run or error
  error:      string | null;
}

/**
 * One-shot: scans every known pool, finds all Active proposals whose
 * voting window has closed without reaching quorum, and calls
 * finalizeProposal on each one.  Respects DRY_RUN.  Prints a summary
 * table then returns — never starts the polling loop.
 */
async function finalizeExpired(
  pools:      PublicKey[],
  govProgram: Program,
  executor:   Keypair
): Promise<void> {
  const now = Math.floor(Date.now() / 1000);

  console.log("\n╔═══════════════════════════════════════════════════════╗");
  console.log("║   WarpXSwap — Finalize Expired Proposals              ║");
  console.log("╚═══════════════════════════════════════════════════════╝\n");
  console.log(`  RPC:      ${RPC_URL}`);
  console.log(`  Executor: ${executor.publicKey.toBase58()}`);
  console.log(`  Pools:    ${pools.length}`);
  console.log(`  DRY RUN:  ${DRY_RUN}`);
  console.log(`  Time:     ${new Date().toISOString()}\n`);

  const results: FinalizeResult[] = [];

  for (const poolKey of pools) {
    const poolShort = poolKey.toBase58().slice(0, 8) + "…";
    const [govPda]  = findGovernancePDA(poolKey, GOVERNANCE_PROGRAM_ID);

    let govAccount: { proposalCount: BN };
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      govAccount = await (govProgram.account as any).governanceAccount.fetch(govPda) as any;
    } catch {
      logWarn(`Pool ${poolShort} — governance PDA not found, skipping`);
      continue;
    }

    const count = govAccount.proposalCount.toNumber();

    for (let i = 0; i < count; i++) {
      const proposalId = new BN(i);
      const [proposalPda] = PublicKey.findProgramAddressSync(
        [
          Buffer.from("proposal"),
          poolKey.toBuffer(),
          proposalId.toArrayLike(Buffer, "le", 8),
        ],
        GOVERNANCE_PROGRAM_ID
      );

      let proposal: {
        status:    { active?: object; passed?: object; executed?: object; rejected?: object; pendingEmergencyApproval?: object };
        endsAt:    BN;
        votesYes:  BN;
        votesNo:   BN;
        payload:   object;
      };
      try {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        proposal = await (govProgram.account as any).proposalAccount.fetch(proposalPda) as any;
      } catch {
        continue;
      }

      const statusKey = Object.keys(proposal.status)[0];
      if (statusKey !== "active") continue;

      const endsAt = proposal.endsAt.toNumber();
      if (endsAt >= now) continue;   // still within voting window

      const yes    = proposal.votesYes.toNumber();
      const no     = proposal.votesNo.toNumber();
      const total  = yes + no;
      const yesBps = total > 0 ? Math.floor((yes * 10_000) / total) : 0;
      const yesPct = (yesBps / 100).toFixed(1) + "%";
      const type   = payloadTypeName(proposal.payload);

      const row: FinalizeResult = {
        pool: poolShort, proposalId: i, type, yes, no, yesPct, tx: null, error: null,
      };

      if (DRY_RUN) {
        logInfo(`  [DRY RUN] Would finalize pool ${poolShort} proposal #${i} [${type}]  ${yes}Y/${no}N (${yesPct})`);
        row.tx = "dry-run";
        results.push(row);
        continue;
      }

      try {
        const tx = await govProgram.methods
          .finalizeProposal(proposalId)
          .accounts({
            governance: govPda,
            proposal:   proposalPda,
            caller:     executor.publicKey,
          })
          .rpc();
        logInfo(`  ✓ Finalized pool ${poolShort} proposal #${i} [${type}]  ${yes}Y/${no}N (${yesPct})  tx=${tx.slice(0, 20)}…`);
        row.tx = tx;
      } catch (e) {
        const msg = (e as Error).message ?? String(e);
        logError(`  ✗ pool ${poolShort} proposal #${i}: ${msg}`);
        row.error = msg;
      }

      results.push(row);
    }
  }

  // ── Summary table ─────────────────────────────────────────────────
  console.log(`\n${"─".repeat(90)}`);
  if (results.length === 0) {
    console.log("  No expired Active proposals found — nothing to finalize.\n");
    return;
  }

  const succeeded = results.filter(r => r.tx && r.tx !== "dry-run" && !r.error).length;
  const dryCount  = results.filter(r => r.tx === "dry-run").length;
  const failed    = results.filter(r => r.error).length;

  console.log(`  Scanned pools: ${pools.length}   Expired proposals found: ${results.length}`);
  if (DRY_RUN) {
    console.log(`  Would finalize: ${dryCount} (dry run — no transactions sent)`);
  } else {
    console.log(`  Finalized: ${succeeded}   Failed: ${failed}`);
  }
  console.log(`${"─".repeat(90)}\n`);

  console.log(
    `  ${"Pool".padEnd(12)} ` +
    `${"#".padEnd(4)} ` +
    `${"Type".padEnd(16)} ` +
    `${"YES".padStart(5)} ` +
    `${"NO".padStart(5)} ` +
    `${"YES%".padStart(7)}  ` +
    `Result`
  );
  console.log(`  ${"─".repeat(78)}`);

  for (const r of results) {
    const result = r.error
      ? `✗ ${r.error.slice(0, 40)}`
      : r.tx === "dry-run"
      ? "⚡ dry-run"
      : `✓ ${r.tx!.slice(0, 20)}…`;

    console.log(
      `  ${r.pool.padEnd(12)} ` +
      `${String(r.proposalId).padEnd(4)} ` +
      `${r.type.padEnd(16)} ` +
      `${String(r.yes).padStart(5)} ` +
      `${String(r.no).padStart(5)} ` +
      `${r.yesPct.padStart(7)}  ` +
      result
    );
  }
  console.log(`  ${"─".repeat(78)}\n`);
}

// ── MAIN LOOP ─────────────────────────────────────────────────────────

async function runOnce(
  pools:      PublicKey[],
  govProgram: Program,
  executor:   Keypair
): Promise<void> {
  if (pools.length === 0) {
    logDebug("No pools to check");
    return;
  }

  const now = Math.floor(Date.now() / 1000);
  let totalChecked = 0, totalExecuted = 0, totalExpired = 0, totalEmergency = 0;

  await Promise.all(
    pools.map(async (poolKey) => {
      try {
        const stats = await pollPool(poolKey, govProgram, executor, now);
        totalChecked    += stats.checked;
        totalExecuted   += stats.executed;
        totalExpired    += stats.expired;
        totalEmergency  += stats.emergency;
      } catch (e) {
        logError(`Pool ${poolKey.toBase58().slice(0, 8)}…: ${(e as Error).message ?? e}`);
      }
    })
  );

  logInfo(
    `Sweep done — pools=${pools.length} proposals=${totalChecked} ` +
    `executed=${totalExecuted} expired=${totalExpired} emergency=${totalEmergency}`
  );
}

async function main() {
  if (!fs.existsSync(WALLET_PATH)) {
    throw new Error(`Executor wallet not found: ${WALLET_PATH}`);
  }
  const rawKey   = JSON.parse(fs.readFileSync(WALLET_PATH, "utf-8")) as number[];
  const executor = Keypair.fromSecretKey(Uint8Array.from(rawKey));

  const connection = new Connection(RPC_URL, "confirmed");
  const provider   = new AnchorProvider(connection, new Wallet(executor), {
    commitment: "confirmed",
  });

  const govIdl     = loadIdl("governance_program");
  const govProgram = new Program(govIdl, provider);

  const pools = discoverPools();

  // ── QUORUM-CHECK mode: print tallies (once or watch-loop) ──────────
  if (QUORUM_CHECK) {
    if (!WATCH_MODE) {
      await printQuorumReport(pools, govProgram);
      process.exit(0);
    }

    // --watch: clear screen and re-print on every interval
    process.on("SIGINT", () => {
      process.stdout.write("\n\nStopped (Ctrl+C).\n");
      process.exit(0);
    });

    console.log(
      `\nLive quorum dashboard — refreshing every ${WATCH_MS / 1000}s  ` +
      `(Ctrl+C to stop)\n`
    );

    // eslint-disable-next-line no-constant-condition
    while (true) {
      console.clear();
      try {
        await printQuorumReport(pools, govProgram);
      } catch (e) {
        logError(`Quorum refresh error: ${(e as Error).message ?? e}`);
      }
      console.log(`  Next refresh in ${WATCH_MS / 1000}s — press Ctrl+C to stop\n`);
      await new Promise<void>((resolve) => setTimeout(resolve, WATCH_MS));
    }
  }

  // ── FINALIZE-EXPIRED mode: finalize all expired proposals then exit ─
  if (FINALIZE_EXPIRED) {
    await finalizeExpired(pools, govProgram, executor);
    process.exit(0);
  }

  // ── NORMAL CRANK mode ─────────────────────────────────────────────
  console.log("\n╔═══════════════════════════════════════════════════════╗");
  console.log("║   WarpXSwap — Governance Crank                        ║");
  console.log("╚═══════════════════════════════════════════════════════╝\n");
  if (DRY_RUN) console.log("  ⚡ DRY RUN — proposals detected but not executed\n");

  logInfo(`RPC:       ${RPC_URL}`);
  logInfo(`Executor:  ${executor.publicKey.toBase58()}`);
  logInfo(`Interval:  ${GOVERN_MS / 1000}s`);
  logInfo(`\nStarting governance crank — watching ${pools.length} pool(s)…\n`);

  // Initial sweep immediately
  await runOnce(pools, govProgram, executor);

  // Then poll on interval
  const loop = async () => {
    try {
      await runOnce(pools, govProgram, executor);
    } catch (e) {
      logError(`Sweep error: ${(e as Error).message ?? e}`);
    }
    setTimeout(loop, GOVERN_MS);
  };

  setTimeout(loop, GOVERN_MS);

  // Keep process alive
  await new Promise<void>(() => {});
}

main().catch((e) => {
  logError(e.message ?? e);
  process.exit(1);
});
