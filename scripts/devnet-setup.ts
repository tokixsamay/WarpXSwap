#!/usr/bin/env ts-node
// ══════════════════════════════════════════════════════════════════
// devnet-setup — Devnet bootstrap helper
//
// Does everything needed to run WarpXSwap on Solana devnet:
//   1. Airdrops SOL to your wallet (up to 10 SOL total)
//   2. Creates test SPL mints for USDT and USDG (SOL + USDC are canonical)
//   3. Mints test tokens to your wallet
//   4. Saves mint addresses to devnet-mints.json for use in complete-setup.ts
//
// Usage:
//   ts-node scripts/devnet-setup.ts
//   ts-node scripts/devnet-setup.ts --airdrop-only
//   ts-node scripts/devnet-setup.ts --skip-airdrop
//
// Env:
//   RPC_URL=https://api.devnet.solana.com
//   WALLET_PATH=~/.config/solana/id.json
// ══════════════════════════════════════════════════════════════════

import * as fs   from "fs";
import * as os   from "os";
import * as path from "path";
import {
  Connection,
  Keypair,
  PublicKey,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import {
  createMint,
  getOrCreateAssociatedTokenAccount,
  mintTo,
  getMint,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { AnchorProvider, Wallet } from "@coral-xyz/anchor";

const RPC_URL     = process.env.RPC_URL ?? "https://api.devnet.solana.com";
const WALLET_PATH = process.env.WALLET_PATH
  ? path.resolve(process.env.WALLET_PATH)
  : path.join(os.homedir(), ".config", "solana", "id.json");

const AIRDROP_ONLY  = process.argv.includes("--airdrop-only");
const SKIP_AIRDROP  = process.argv.includes("--skip-airdrop");
const MINTS_FILE    = path.join(__dirname, "..", "devnet-mints.json");

// Canonical devnet mint addresses (use these, don't recreate)
const CANONICAL_MINTS = {
  SOL:  "So11111111111111111111111111111111111111112",
  USDC: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
  // USDT and USDG will be created as test mints below
};

// ── Helpers ───────────────────────────────────────────────────────

const C = {
  green:  "\x1b[32m", red:  "\x1b[31m",
  yellow: "\x1b[33m", cyan: "\x1b[36m",
  reset:  "\x1b[0m",  bold: "\x1b[1m", dim: "\x1b[2m",
};

function ok(msg: string)   { console.log(`  ${C.green}✓${C.reset}  ${msg}`); }
function skip(msg: string) { console.log(`  ${C.dim}→  ${msg}${C.reset}`); }
function info(msg: string) { console.log(`  ${C.cyan}ℹ${C.reset}  ${msg}`); }
function head(msg: string) { console.log(`\n${C.bold}${msg}${C.reset}`); }

async function airdrop(
  connection: Connection,
  wallet:     PublicKey,
  targetSol:  number
): Promise<void> {
  const bal = (await connection.getBalance(wallet)) / LAMPORTS_PER_SOL;
  if (bal >= targetSol) {
    skip(`Balance already ${bal.toFixed(3)} SOL (target: ${targetSol} SOL)`);
    return;
  }

  const needed = Math.min(targetSol - bal, 2); // devnet max 2 SOL per airdrop
  const rounds = Math.ceil((targetSol - bal) / 2);

  for (let i = 0; i < Math.min(rounds, 5); i++) {
    try {
      const sig = await connection.requestAirdrop(wallet, Math.min(needed, 2) * LAMPORTS_PER_SOL);
      await connection.confirmTransaction(sig, "confirmed");
      const newBal = (await connection.getBalance(wallet)) / LAMPORTS_PER_SOL;
      ok(`Airdrop round ${i + 1}: +${needed.toFixed(1)} SOL → ${newBal.toFixed(3)} SOL`);
      if (newBal >= targetSol) break;
      await new Promise(r => setTimeout(r, 2000)); // wait between airdrops
    } catch (e) {
      console.warn(`  Airdrop round ${i + 1} failed: ${(e as Error).message}`);
      break;
    }
  }
}

async function getOrCreateTestMint(
  connection: Connection,
  payer:      Keypair,
  label:      string,
  decimals:   number,
  savedMints: Record<string, string>
): Promise<PublicKey> {
  if (savedMints[label]) {
    const existing = new PublicKey(savedMints[label]);
    try {
      await getMint(connection, existing);
      skip(`${label} mint already exists: ${existing.toBase58()}`);
      return existing;
    } catch {
      info(`${label} mint in file not found on-chain — recreating`);
    }
  }

  const mint = await createMint(
    connection,
    payer,
    payer.publicKey,
    payer.publicKey,
    decimals
  );
  ok(`${label} mint created: ${mint.toBase58()} (${decimals} decimals)`);
  return mint;
}

async function mintTokensToWallet(
  connection: Connection,
  payer:      Keypair,
  mint:       PublicKey,
  label:      string,
  amount:     bigint
): Promise<void> {
  const ata = await getOrCreateAssociatedTokenAccount(
    connection, payer, mint, payer.publicKey
  );
  await mintTo(connection, payer, mint, ata.address, payer, amount);
  ok(`Minted ${(Number(amount) / 1e6).toLocaleString()} ${label} to ${ata.address.toBase58()}`);
}

// ── Main ──────────────────────────────────────────────────────────

async function main() {
  console.log(`\n${C.bold}${C.cyan}WarpXSwap Devnet Setup${C.reset}`);
  console.log(`${C.dim}RPC: ${RPC_URL}${C.reset}\n`);

  if (!fs.existsSync(WALLET_PATH)) {
    console.error(`✗ Wallet not found: ${WALLET_PATH}`);
    console.error(`  Run: solana-keygen new --outfile ${WALLET_PATH}`);
    process.exit(1);
  }

  const raw        = JSON.parse(fs.readFileSync(WALLET_PATH, "utf-8")) as number[];
  const payer      = Keypair.fromSecretKey(Uint8Array.from(raw));
  const connection = new Connection(RPC_URL, "confirmed");

  info(`Wallet: ${payer.publicKey.toBase58()}`);

  // ── 1. Airdrop ─────────────────────────────────────────────────
  if (!SKIP_AIRDROP) {
    head("1. Airdrop SOL");
    await airdrop(connection, payer.publicKey, 10);
  } else {
    skip("Airdrop skipped (--skip-airdrop)");
  }

  if (AIRDROP_ONLY) {
    const bal = (await connection.getBalance(payer.publicKey)) / LAMPORTS_PER_SOL;
    ok(`Final balance: ${bal.toFixed(4)} SOL`);
    return;
  }

  // ── 2. Load existing mint file ─────────────────────────────────
  let savedMints: Record<string, string> = {};
  if (fs.existsSync(MINTS_FILE)) {
    savedMints = JSON.parse(fs.readFileSync(MINTS_FILE, "utf-8"));
    info(`Loaded existing mint addresses from devnet-mints.json`);
  }

  // ── 3. Create test mints ───────────────────────────────────────
  head("2. Create Test Mints");
  info("SOL and USDC use canonical devnet addresses (no creation needed)");

  const usdtMint = await getOrCreateTestMint(connection, payer, "USDT", 6, savedMints);
  const usdgMint = await getOrCreateTestMint(connection, payer, "USDG", 6, savedMints);

  // ── 4. Mint tokens to wallet ───────────────────────────────────
  head("3. Mint Test Tokens to Wallet");
  await mintTokensToWallet(connection, payer, usdtMint, "USDT", 1_000_000_000_000n); // 1M USDT
  await mintTokensToWallet(connection, payer, usdgMint, "USDG", 1_000_000_000_000n); // 1M USDG

  // ── 5. Save addresses ──────────────────────────────────────────
  head("4. Save Mint Addresses");
  const mints = {
    ...savedMints,
    SOL:  CANONICAL_MINTS.SOL,
    USDC: CANONICAL_MINTS.USDC,
    USDT: usdtMint.toBase58(),
    USDG: usdgMint.toBase58(),
    network:   "devnet",
    createdAt: new Date().toISOString(),
    wallet:    payer.publicKey.toBase58(),
  };

  fs.writeFileSync(MINTS_FILE, JSON.stringify(mints, null, 2));
  ok(`Saved to devnet-mints.json`);

  // ── 6. Summary ─────────────────────────────────────────────────
  const finalBal = (await connection.getBalance(payer.publicKey)) / LAMPORTS_PER_SOL;

  console.log(`
${C.bold}${C.green}✅ Devnet setup complete!${C.reset}

  Wallet balance: ${finalBal.toFixed(4)} SOL

  Mint addresses (devnet-mints.json):
  ${C.dim}SOL : ${mints.SOL}
  USDC: ${mints.USDC}
  USDT: ${mints.USDT}
  USDG: ${mints.USDG}${C.reset}

  Next steps:
  ${C.cyan}1.${C.reset} anchor build
  ${C.cyan}2.${C.reset} ts-node scripts/update-program-ids.ts --network devnet
  ${C.cyan}3.${C.reset} anchor build  (rebuild with new IDs)
  ${C.cyan}4.${C.reset} anchor deploy --provider.cluster devnet
  ${C.cyan}5.${C.reset} USDT_MINT=${mints.USDT} USDG_MINT=${mints.USDG} ts-node scripts/complete-setup.ts
  ${C.cyan}6.${C.reset} ts-node scripts/verify-deploy.ts
  ${C.cyan}7.${C.reset} RPC_URL=https://api.devnet.solana.com ts-node scripts/crank.ts
`);
}

main().catch((e) => {
  console.error(`\nFatal: ${e.message}\n`);
  process.exit(1);
});
