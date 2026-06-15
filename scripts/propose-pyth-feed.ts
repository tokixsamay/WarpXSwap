#!/usr/bin/env ts-node
// ╔═══════════════════════════════════════════════════════════════════╗
// ║   WarpXSwap — Propose-Pyth-Feed CLI                              ║
// ║                                                                   ║
// ║   Stages a SetPythFeedId governance proposal.                     ║
// ║   The new 32-byte Pyth V2 feed ID is written to the AssetInfo    ║
// ║   on-chain after the proposal passes and is executed.            ║
// ║                                                                   ║
// ║   After the 48-hour voting window and >51% YES votes,            ║
// ║   run:  ts-node scripts/govern-crank.ts                           ║
// ║   or call execute_proposal directly to apply the change.         ║
// ╚═══════════════════════════════════════════════════════════════════╝
//
// Usage:
//   ts-node scripts/propose-pyth-feed.ts \
//     --pool  <pool-PDA>          Pool PDA address (base58) \
//     --mint  <mint-pubkey>       Token mint whose feed ID to rotate \
//     --feed  <hex>               New 32-byte Pyth feed ID (hex, with or without 0x) \
//     [--vote]                    Also cast a YES vote immediately after proposing \
//     [--no]                      Also cast a NO vote immediately after proposing \
//     [--emergency]               Submit as emergency proposal (Top-10 path) \
//     [--dry-run]                 Print the payload without sending a transaction
//
// Env vars:
//   RPC_URL=http://127.0.0.1:8899
//   WALLET_PATH=~/.config/solana/id.json   (proposer; must be a registered contributor)

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
} from "../sdk/src";

// ── CLI ARGS ─────────────────────────────────────────────────────────

function parseArgs(): {
  poolPda:     PublicKey;
  mint:        PublicKey;
  feedHex:     string;
  voteYes:     boolean;
  voteNo:      boolean;
  emergency:   boolean;
  dryRun:      boolean;
} {
  const args = process.argv.slice(2);
  const get  = (flag: string): string | undefined => {
    const i = args.indexOf(flag);
    return i >= 0 ? args[i + 1] : undefined;
  };

  const poolStr  = get("--pool");
  const mintStr  = get("--mint");
  const feedStr  = get("--feed");
  const voteYes   = args.includes("--vote");
  const voteNo    = args.includes("--no");
  const emergency = args.includes("--emergency");
  const dryRun    = args.includes("--dry-run");

  if (!poolStr) throw new Error("Missing --pool <pool-PDA>");
  if (!mintStr) throw new Error("Missing --mint <mint-pubkey>");
  if (!feedStr) throw new Error("Missing --feed <32-byte-hex>");
  if (voteYes && voteNo) throw new Error("--vote and --no are mutually exclusive");

  return {
    poolPda:   new PublicKey(poolStr),
    mint:      new PublicKey(mintStr),
    feedHex:   feedStr.replace(/^0x/, ""),
    voteYes,
    voteNo,
    emergency,
    dryRun,
  };
}

function hexToBytes32(hex: string): number[] {
  if (hex.length !== 64) {
    throw new Error(
      `Pyth feed ID must be exactly 64 hex chars (32 bytes), got ${hex.length} chars: ${hex}`
    );
  }
  const bytes: number[] = [];
  for (let i = 0; i < 64; i += 2) {
    bytes.push(parseInt(hex.slice(i, i + 2), 16));
  }
  return bytes;
}

// ── HELPERS ───────────────────────────────────────────────────────────

const IDL_DIR = path.join(__dirname, "..", "target", "idl");

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function loadIdl(name: string): any {
  const p = path.join(IDL_DIR, `${name}.json`);
  if (!fs.existsSync(p)) {
    throw new Error(`IDL not found: ${p}\nRun 'anchor build' first.`);
  }
  return JSON.parse(fs.readFileSync(p, "utf-8"));
}

// ── MAIN ──────────────────────────────────────────────────────────────

async function main() {
  const { poolPda, mint, feedHex, voteYes, voteNo, emergency, dryRun } = parseArgs();

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

  // Derive PDAs
  const [governancePda] = findGovernancePDA(poolPda, GOVERNANCE_PROGRAM_ID);
  const [contributorPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("contributor"), poolPda.toBuffer(), proposer.publicKey.toBuffer()],
    GOVERNANCE_PROGRAM_ID
  );

  // Parse and validate feed ID
  const feedIdBytes = hexToBytes32(feedHex);

  // ── PREVIEW ──────────────────────────────────────────────────────
  console.log("\n╔═══════════════════════════════════════════════════════╗");
  console.log("║   WarpXSwap — SetPythFeedId Proposal                  ║");
  console.log("╚═══════════════════════════════════════════════════════╝\n");
  console.log(`RPC:          ${RPC_URL}`);
  console.log(`Proposer:     ${proposer.publicKey.toBase58()}`);
  console.log(`Pool PDA:     ${poolPda.toBase58()}`);
  console.log(`Governance:   ${governancePda.toBase58()}`);
  console.log(`Contributor:  ${contributorPda.toBase58()}`);
  console.log(`Mint:         ${mint.toBase58()}`);
  console.log(`New Feed ID:  0x${feedHex}`);
  console.log(`Emergency:    ${emergency}`);
  console.log(`Auto-vote:    ${voteYes ? "YES (proposer will self-vote immediately)" : voteNo ? "NO (proposer signals dissent immediately)" : "none"}`);

  if (dryRun) {
    console.log("\n⚡ DRY RUN — no transaction sent.\n");
    console.log("Payload that would be submitted:");
    console.log(JSON.stringify({
      proposalType: "SetPythFeedId",
      payload: {
        mint: mint.toBase58(),
        pythFeedId: feedHex,
      },
      isEmergency: emergency,
      autoVote: voteYes ? "YES" : voteNo ? "NO" : null,
    }, null, 2));
    return;
  }

  // ── VERIFY CONTRIBUTOR ACCOUNT EXISTS ────────────────────────────
  const contributorInfo = await connection.getAccountInfo(contributorPda);
  if (!contributorInfo) {
    throw new Error(
      `Contributor account not found: ${contributorPda.toBase58()}\n` +
      `Register first via register_contributor (in complete-setup.ts or CLI).`
    );
  }

  // ── FETCH GOVERNANCE (to get current proposal_count) ─────────────
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const govAccount: any = await (govProgram.account as any).governanceAccount.fetch(governancePda);
  const proposalId: BN  = govAccount.proposalCount as BN;

  // Derive upcoming proposal PDA
  const [proposalPda] = PublicKey.findProgramAddressSync(
    [
      Buffer.from("proposal"),
      poolPda.toBuffer(),
      proposalId.toArrayLike(Buffer, "le", 8),
    ],
    GOVERNANCE_PROGRAM_ID
  );

  console.log(`\nNext Proposal ID: ${proposalId.toString()}`);
  console.log(`Proposal PDA:     ${proposalPda.toBase58()}`);
  console.log("\nSubmitting transaction…");

  // ── SEND PROPOSAL TX ─────────────────────────────────────────────
  const tx = await govProgram.methods
    .createProposal(
      { setPythFeedId: {} },                         // ProposalType::SetPythFeedId
      { setPythFeedId: { mint, pythFeedId: feedIdBytes } }, // ProposalPayload::SetPythFeedId
      emergency
    )
    .accounts({
      governance:   governancePda,
      proposal:     proposalPda,
      contributor:  contributorPda,
      proposer:     proposer.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  console.log("\n✓ Proposal submitted!");
  console.log(`  Proposal ID:  ${proposalId.toString()}`);
  console.log(`  Proposal PDA: ${proposalPda.toBase58()}`);
  console.log(`  Tx signature: ${tx}`);

  // ── SELF-VOTE (--vote / --no flag) ───────────────────────────────
  if (voteYes || voteNo) {
    const voteValue = voteYes;
    const voteLabel = voteYes ? "YES" : "NO";
    console.log(`\nCasting ${voteLabel} vote as proposer…`);
    const voteTx = await govProgram.methods
      .castVote(proposalId, voteValue)
      .accounts({
        governance:  governancePda,
        proposal:    proposalPda,
        contributor: contributorPda,
        voter:       proposer.publicKey,
      })
      .rpc();
    console.log(`✓ ${voteLabel} vote recorded!`);
    console.log(`  Tx signature: ${voteTx}`);
  }

  console.log("\nNext steps:");
  if (emergency) {
    console.log("  1. Top-10 contributors must call approve_emergency (6 of 10 needed)");
    console.log("  2. Once approved, remaining contributors cast_vote during the 48h window");
  } else {
    const voteNote = (voteYes || voteNo) ? "remaining contributors cast" : "Contributors cast";
    console.log(`  1. ${voteNote} votes during the 48h window (>51% YES to pass)`);
  }
  console.log("  2. After passing, run:  ts-node scripts/govern-crank.ts");
  console.log("     to auto-execute all Passed proposals (including this one).\n");
}

main().catch((e) => {
  console.error("\n✗ Error:", e.message ?? e);
  process.exit(1);
});
