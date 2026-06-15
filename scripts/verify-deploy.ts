#!/usr/bin/env ts-node
// ══════════════════════════════════════════════════════════════════
// verify-deploy — Post-deployment verification script
//
// Checks that all 4 programs are deployed and executable on-chain.
//
// Usage:
//   RPC_URL=https://api.devnet.solana.com ts-node scripts/verify-deploy.ts
//   RPC_URL=https://api.mainnet-beta.solana.com ts-node scripts/verify-deploy.ts
// ══════════════════════════════════════════════════════════════════

import * as os   from "os";
import * as path from "path";
import * as fs   from "fs";
import { Connection, PublicKey, Keypair } from "@solana/web3.js";
import { AnchorProvider, Program, Wallet } from "@coral-xyz/anchor";
import {
  POOL_PROGRAM_ID,
  INFO_POOL_PROGRAM_ID,
  GOVERNANCE_PROGRAM_ID,
  ROUTING_PROGRAM_ID,
  findPoolPDA,
  findInfoPoolPDA,
  findGovernancePDA,
} from "../sdk/src";
import { createPrograms } from "../sdk/src/idl-loader";

const RPC_URL     = process.env.RPC_URL ?? "https://api.devnet.solana.com";
const WALLET_PATH = process.env.WALLET_PATH
  ? path.resolve(process.env.WALLET_PATH)
  : path.join(os.homedir(), ".config", "solana", "id.json");

// ── Colors ────────────────────────────────────────────────────────
const C = {
  green:  "\x1b[32m",
  red:    "\x1b[31m",
  yellow: "\x1b[33m",
  cyan:   "\x1b[36m",
  reset:  "\x1b[0m",
  bold:   "\x1b[1m",
  dim:    "\x1b[2m",
};

function pass(msg: string)  { console.log(`  ${C.green}✓${C.reset}  ${msg}`); }
function fail(msg: string)  { console.log(`  ${C.red}✗${C.reset}  ${msg}`); }
function warn(msg: string)  { console.log(`  ${C.yellow}⚠${C.reset}  ${msg}`); }
function info(msg: string)  { console.log(`  ${C.dim}${msg}${C.reset}`); }
function head(msg: string)  { console.log(`\n${C.bold}${C.cyan}${msg}${C.reset}`); }

let passCount = 0;
let failCount = 0;

function check(ok: boolean, msg: string) {
  if (ok) { pass(msg); passCount++; }
  else     { fail(msg); failCount++; }
}

// ── Checks ────────────────────────────────────────────────────────

async function checkProgramDeployed(
  connection: Connection,
  programId:  PublicKey,
  name:       string
): Promise<boolean> {
  try {
    const info = await connection.getAccountInfo(programId);
    if (!info) {
      check(false, `${name} — account not found on-chain`);
      return false;
    }
    if (!info.executable) {
      check(false, `${name} — account exists but NOT executable`);
      return false;
    }
    check(true, `${name} deployed — ${programId.toBase58()}`);
    return true;
  } catch (e) {
    check(false, `${name} — RPC error: ${(e as Error).message}`);
    return false;
  }
}

async function checkBalance(connection: Connection, wallet: PublicKey) {
  const lamports = await connection.getBalance(wallet);
  const sol      = lamports / 1e9;
  if (sol < 0.05) {
    warn(`Wallet balance LOW: ${sol.toFixed(4)} SOL — crank will fail soon`);
  } else {
    pass(`Wallet balance: ${sol.toFixed(4)} SOL`);
  }
  return sol;
}

async function checkPdaExists(
  connection: Connection,
  pda:        PublicKey,
  label:      string
): Promise<boolean> {
  const acct = await connection.getAccountInfo(pda);
  if (acct) {
    check(true,  `${label} PDA initialized — ${pda.toBase58().slice(0, 16)}...`);
    return true;
  } else {
    warn(`${label} PDA not found — run complete-setup.ts`);
    return false;
  }
}

async function checkIdlFiles(): Promise<boolean> {
  const idlDir  = path.join(__dirname, "..", "target", "idl");
  const sdkIdl  = path.join(__dirname, "..", "sdk", "idl");
  const programs = ["pool_program", "info_pool_program", "governance_program", "routing_program"];

  let allFound = true;
  for (const prog of programs) {
    const inTarget = fs.existsSync(path.join(idlDir,  `${prog}.json`));
    const inSdk    = fs.existsSync(path.join(sdkIdl,  `${prog}.json`));
    if (inTarget || inSdk) {
      check(true, `IDL: ${prog}.json ${inTarget ? "(target/idl)" : "(sdk/idl)"}`);
    } else {
      check(false, `IDL: ${prog}.json — not found. Run \`anchor build\``);
      allFound = false;
    }
  }
  return allFound;
}

// ── Main ──────────────────────────────────────────────────────────

async function main() {
  console.log(`\n${C.bold}${C.cyan}WarpXSwap Deploy Verification${C.reset}`);
  console.log(`${C.dim}RPC: ${RPC_URL}${C.reset}\n`);

  const connection = new Connection(RPC_URL, "confirmed");

  // ── 1. Network ─────────────────────────────────────────────────
  head("1. Network");
  try {
    const slot = await connection.getSlot();
    pass(`Connected to cluster — slot ${slot}`);
  } catch {
    fail(`Cannot connect to ${RPC_URL}`);
    process.exit(1);
  }

  // ── 2. Programs ────────────────────────────────────────────────
  head("2. Programs On-Chain");
  const poolOk = await checkProgramDeployed(connection, POOL_PROGRAM_ID,       "Pool");
  await checkProgramDeployed(connection, INFO_POOL_PROGRAM_ID,  "InfoPool");
  await checkProgramDeployed(connection, GOVERNANCE_PROGRAM_ID, "Governance");
  await checkProgramDeployed(connection, ROUTING_PROGRAM_ID,    "Routing");

  // ── 3. Wallet ──────────────────────────────────────────────────
  head("3. Wallet");
  let wallet: PublicKey | null = null;
  if (fs.existsSync(WALLET_PATH)) {
    const raw = JSON.parse(fs.readFileSync(WALLET_PATH, "utf-8")) as number[];
    const kp  = Keypair.fromSecretKey(Uint8Array.from(raw));
    wallet    = kp.publicKey;
    pass(`Wallet loaded — ${wallet.toBase58()}`);
    await checkBalance(connection, wallet);
  } else {
    warn(`Wallet not found at ${WALLET_PATH} — skipping balance check`);
  }

  // ── 4. IDL Files ───────────────────────────────────────────────
  head("4. IDL Files");
  await checkIdlFiles();

  // ── 5. PDAs (only if wallet + programs found) ──────────────────
  if (poolOk && wallet) {
    head("5. PDA Accounts");
    const provider = new AnchorProvider(
      connection,
      new Wallet(Keypair.generate()),
      { commitment: "confirmed" }
    );

    try {
      const programs = createPrograms(provider);
      const [poolPda]  = findPoolPDA(wallet, POOL_PROGRAM_ID);
      const [infoPda]  = findInfoPoolPDA(poolPda, INFO_POOL_PROGRAM_ID);
      const [govPda]   = findGovernancePDA(poolPda, GOVERNANCE_PROGRAM_ID);

      await checkPdaExists(connection, poolPda,  "Pool");
      await checkPdaExists(connection, infoPda,  "InfoPool");
      await checkPdaExists(connection, govPda,   "Governance");
      void programs; // silence unused warning
    } catch (e) {
      warn(`Could not check PDAs: ${(e as Error).message}`);
    }
  } else {
    warn("Skipping PDA check (programs not deployed or wallet missing)");
  }

  // ── 6. Summary ─────────────────────────────────────────────────
  head("Summary");
  const total = passCount + failCount;
  console.log(`\n  ${C.green}${passCount}${C.reset} passed  ${C.red}${failCount}${C.reset} failed  (${total} checks)\n`);

  if (failCount === 0) {
    console.log(`  ${C.green}${C.bold}✅ Deploy verified! Ready to run cranks.${C.reset}\n`);
    console.log(`  Next steps:`);
    console.log(`    ts-node scripts/crank.ts        # start main crank`);
    console.log(`    ts-node scripts/govern-crank.ts # start governance crank\n`);
  } else {
    console.log(`  ${C.yellow}⚠  ${failCount} check(s) failed. Review above output.${C.reset}\n`);
    process.exit(1);
  }
}

main().catch((e) => {
  console.error(`\nFatal error: ${e.message}\n`);
  process.exit(1);
});
