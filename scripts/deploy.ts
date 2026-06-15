#!/usr/bin/env ts-node
// ══════════════════════════════════════════════════════════════════
// deploy — Full automated deployment script
//
// Steps:
//   1. Check prerequisites (solana-cli, anchor, wallet)
//   2. Check wallet balance (warn if low)
//   3. Generate program keypairs (if not exist)
//   4. anchor build
//   5. update-program-ids (patch lib.rs + Anchor.toml + constants.ts)
//   6. anchor build again (with correct program IDs)
//   7. anchor deploy (each program)
//   8. Sync IDL to sdk/idl/
//   9. Run complete-setup.ts
//  10. verify-deploy
//
// Usage:
//   ts-node scripts/deploy.ts [--network devnet|mainnet-beta] [--dry-run]
//   ts-node scripts/deploy.ts --skip-build   # skip anchor build steps
//   ts-node scripts/deploy.ts --skip-setup   # skip complete-setup.ts
// ══════════════════════════════════════════════════════════════════

import * as fs          from "fs";
import * as os          from "os";
import * as path        from "path";
import * as child_proc  from "child_process";

const ROOT        = path.resolve(__dirname, "..");
const NETWORK     = (() => {
  const idx = process.argv.indexOf("--network");
  return idx !== -1 ? process.argv[idx + 1] : "devnet";
})();
const DRY_RUN     = process.argv.includes("--dry-run");
const SKIP_BUILD  = process.argv.includes("--skip-build");
const SKIP_SETUP  = process.argv.includes("--skip-setup");

const WALLET_PATH = process.env.WALLET_PATH
  ? path.resolve(process.env.WALLET_PATH)
  : path.join(os.homedir(), ".config", "solana", "id.json");

const PROGRAMS = [
  "pool_program",
  "info_pool_program",
  "governance_program",
  "routing_program",
];

// ── Colors ────────────────────────────────────────────────────────
const C = {
  green:  "\x1b[32m", red:    "\x1b[31m", yellow: "\x1b[33m",
  cyan:   "\x1b[36m", reset:  "\x1b[0m",  bold:   "\x1b[1m",
  dim:    "\x1b[2m",  blue:   "\x1b[34m",
};

function ok(msg: string)   { console.log(`  ${C.green}✓${C.reset}  ${msg}`); }
function fail(msg: string) { console.log(`  ${C.red}✗${C.reset}  ${msg}`); }
function warn(msg: string) { console.log(`  ${C.yellow}⚠${C.reset}  ${msg}`); }
function info(msg: string) { console.log(`  ${C.cyan}ℹ${C.reset}  ${msg}`); }
function head(msg: string) { console.log(`\n${C.bold}${C.cyan}── ${msg} ──${C.reset}\n`); }
function step(n: number, msg: string) {
  console.log(`\n${C.bold}${C.blue}[${n}/10]${C.reset} ${C.bold}${msg}${C.reset}`);
}

function run(
  cmd:   string,
  label: string,
  opts:  { cwd?: string; fatal?: boolean } = {}
): boolean {
  info(`Running: ${C.dim}${cmd}${C.reset}`);
  if (DRY_RUN) {
    ok(`[dry-run] ${label}`);
    return true;
  }
  try {
    child_proc.execSync(cmd, {
      cwd:   opts.cwd ?? ROOT,
      stdio: "inherit",
      env:   { ...process.env },
    });
    ok(label);
    return true;
  } catch {
    if (opts.fatal !== false) {
      fail(label);
    } else {
      warn(`${label} (non-fatal)`);
    }
    return false;
  }
}

function cmdExists(cmd: string): boolean {
  try {
    child_proc.execSync(`which ${cmd}`, { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function cmdVersion(cmd: string): string {
  try {
    return child_proc.execSync(`${cmd} --version`, { encoding: "utf-8" }).trim();
  } catch {
    return "(unknown)";
  }
}

// ── Steps ─────────────────────────────────────────────────────────

function checkPrereqs(): boolean {
  let ok_ = true;

  if (cmdExists("solana")) {
    ok(`solana-cli: ${cmdVersion("solana")}`);
  } else {
    fail("solana-cli not found — install from https://docs.solana.com/cli/install-solana-cli-tools");
    ok_ = false;
  }

  if (cmdExists("anchor")) {
    ok(`anchor-cli: ${cmdVersion("anchor")}`);
  } else {
    fail("anchor-cli not found — run: cargo install --git https://github.com/coral-xyz/anchor avm && avm install 0.31.0");
    ok_ = false;
  }

  if (cmdExists("cargo")) {
    ok(`cargo: ${cmdVersion("cargo")}`);
  } else {
    fail("cargo not found — install Rust from https://rustup.rs");
    ok_ = false;
  }

  if (fs.existsSync(WALLET_PATH)) {
    ok(`Wallet: ${WALLET_PATH}`);
  } else {
    fail(`Wallet not found: ${WALLET_PATH}\n  Run: solana-keygen new --outfile ${WALLET_PATH}`);
    ok_ = false;
  }

  return ok_;
}

function checkWalletBalance(): void {
  try {
    const out = child_proc.execSync(
      `solana balance --url ${NETWORK === "mainnet-beta" ? "mainnet-beta" : "devnet"}`,
      { encoding: "utf-8", cwd: ROOT }
    ).trim();
    const sol = parseFloat(out);
    if (NETWORK === "mainnet-beta" && sol < 10) {
      warn(`Mainnet wallet balance: ${out} — need ~10–15 SOL for 4 programs`);
    } else if (NETWORK === "devnet" && sol < 5) {
      warn(`Devnet balance low: ${out} — run: ts-node scripts/devnet-setup.ts --airdrop-only`);
    } else {
      ok(`Wallet balance: ${out}`);
    }
  } catch {
    warn("Could not check wallet balance");
  }
}

function generateKeypairs(): void {
  const deployDir = path.join(ROOT, "target", "deploy");
  fs.mkdirSync(deployDir, { recursive: true });

  for (const prog of PROGRAMS) {
    const kpFile = path.join(deployDir, `${prog}-keypair.json`);
    if (fs.existsSync(kpFile)) {
      skip(`${prog}-keypair.json already exists`);
    } else {
      run(
        `solana-keygen new --outfile ${kpFile} --no-bip39-passphrase --force`,
        `Generated ${prog}-keypair.json`
      );
    }
  }
}

function skip(msg: string) { console.log(`  ${C.dim}→  ${msg}${C.reset}`); }

// ── Main ──────────────────────────────────────────────────────────

async function main() {
  console.log(`\n${C.bold}${C.cyan}WarpXSwap Deployment${C.reset}`);
  console.log(`${C.dim}Network: ${NETWORK}${DRY_RUN ? " [DRY RUN]" : ""}${C.reset}`);

  // Step 1: Prerequisites
  step(1, "Check prerequisites");
  const prereqsOk = checkPrereqs();
  if (!prereqsOk) {
    console.error(`\n${C.red}✗ Prerequisites missing. Fix above errors first.${C.reset}\n`);
    process.exit(1);
  }

  // Step 2: Wallet balance
  step(2, "Check wallet balance");
  checkWalletBalance();

  // Step 3: Generate keypairs
  step(3, "Generate program keypairs");
  generateKeypairs();

  // Step 4: First anchor build (to create target/ structure)
  if (!SKIP_BUILD) {
    step(4, "anchor build (first pass)");
    if (!run("anchor build", "Programs compiled")) {
      console.error(`\n${C.red}✗ Build failed. Check Rust errors above.${C.reset}\n`);
      process.exit(1);
    }
  } else {
    step(4, "anchor build (SKIPPED)");
    skip("--skip-build flag set");
  }

  // Step 5: Update program IDs from keypairs
  step(5, "Update program IDs");
  run(
    `ts-node ${path.join(__dirname, "update-program-ids.ts")} --network ${NETWORK}`,
    "Program IDs updated in Anchor.toml + lib.rs + constants.ts"
  );

  // Step 6: Second anchor build (with correct program IDs in declare_id!)
  if (!SKIP_BUILD) {
    step(6, "anchor build (second pass — with correct program IDs)");
    if (!run("anchor build", "Programs recompiled with correct IDs")) {
      console.error(`\n${C.red}✗ Rebuild failed. Check Rust errors above.${C.reset}\n`);
      process.exit(1);
    }
  } else {
    step(6, "anchor build second pass (SKIPPED)");
    skip("--skip-build flag set");
  }

  // Step 7: Deploy all programs
  step(7, "anchor deploy");
  for (const prog of PROGRAMS) {
    const success = run(
      `anchor deploy --program-name ${prog} --provider.cluster ${NETWORK}`,
      `Deployed ${prog}`,
      { fatal: false }
    );
    if (!success) {
      warn(`${prog} deploy failed — will continue with others`);
    }
  }

  // Step 8: Sync IDL to sdk/idl/
  step(8, "Sync IDL snapshots");
  run(
    `ts-node ${path.join(ROOT, "sdk", "src", "sync-idl.ts")}`,
    "IDL files synced to sdk/idl/"
  );

  // Step 9: Run complete-setup.ts
  if (!SKIP_SETUP) {
    step(9, "Run complete-setup.ts");
    const rpcFlag = `RPC_URL=https://api.${NETWORK === "mainnet-beta" ? "mainnet-beta" : "devnet"}.solana.com`;
    const setupOk = run(
      `${rpcFlag} ts-node ${path.join(__dirname, "complete-setup.ts")}`,
      "Pools, InfoPools, and Governance initialized",
      { fatal: false }
    );
    if (!setupOk) {
      warn("Setup failed — verify step will likely fail too. Fix and re-run complete-setup.ts manually.");
    }
  } else {
    step(9, "complete-setup.ts (SKIPPED)");
    skip("--skip-setup flag set");
  }

  // Step 10: Verify
  step(10, "Verify deployment");
  run(
    `RPC_URL=https://api.${NETWORK === "mainnet-beta" ? "mainnet-beta" : "devnet"}.solana.com ts-node ${path.join(__dirname, "verify-deploy.ts")}`,
    "Deployment verified"
  );

  console.log(`
${C.bold}${C.green}🚀 Deployment complete!${C.reset}

  Start cranks (run in separate terminals):
  ${C.cyan}Terminal 1:${C.reset} RPC_URL=https://api.${NETWORK}.solana.com ts-node scripts/crank.ts
  ${C.cyan}Terminal 2:${C.reset} RPC_URL=https://api.${NETWORK}.solana.com ts-node scripts/govern-crank.ts
`);
}

main().catch((e) => {
  console.error(`\nFatal: ${e.message}\n`);
  process.exit(1);
});
