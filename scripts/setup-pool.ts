/**
 * WarpXSwap — Pool Setup Script
 *
 * Pura pool creation flow ek shot mein:
 *   1. Pool Program initialize
 *   2. Har asset register (fee range, threshold, allowed list)
 *   3. InfoPool initialize (Pyth / IL tracking shadow)
 *   4. Initial liquidity deposit (optional)
 *
 * Usage:
 *   ts-node scripts/setup-pool.ts
 *
 * Environment variables (required):
 *   ANCHOR_WALLET   — path to LP keypair JSON  (default: ~/.config/solana/id.json)
 *   ANCHOR_PROVIDER_URL — RPC URL              (default: http://127.0.0.1:8899)
 *
 * Example (localnet):
 *   ANCHOR_WALLET=./keypairs/lp.json \
 *   ANCHOR_PROVIDER_URL=http://127.0.0.1:8899 \
 *   ts-node scripts/setup-pool.ts
 */

import * as anchor from "@coral-xyz/anchor";
import { BN, AnchorProvider, Program } from "@coral-xyz/anchor";
import {
  Connection,
  Keypair,
  PublicKey,
  clusterApiUrl,
} from "@solana/web3.js";
import {
  getOrCreateAssociatedTokenAccount,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import * as fs from "fs";
import * as path from "path";
import * as os from "os";

import { PoolSetupClient } from "../sdk/src/pool-setup";
import type { PoolSetupConfig } from "../sdk/src/pool-setup";
import { findPoolPDA, findAssetPDA } from "../sdk/src/pda";

// ═══════════════════════════════════════════════════
// CONFIG — apni values yahan daal do
// ═══════════════════════════════════════════════════

// Token mints — devnet/localnet ke liye apne test mints use karo
// Mainnet par actual mint addresses daal do
const SOL_MINT  = new PublicKey("So11111111111111111111111111111111111111112");
const ETH_MINT  = new PublicKey("7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs"); // devnet wETH
const USDC_MINT = new PublicKey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"); // devnet USDC

// Initial deposit amounts (0 = skip deposit, just setup)
const SOL_DEPOSIT_AMOUNT  = new BN(10_000_000_000); // 10 SOL  (lamports)
const ETH_DEPOSIT_AMOUNT  = new BN(5_000_000_000);  // 5 units (8-dec scale)
const USDC_DEPOSIT_AMOUNT = new BN(0);               // skip

// Pyth-scale initial prices (price × 10^8)
// Devnet/test mein koi bhi value daal sakte ho
const SOL_BASE_PRICE  = new BN(8_600_000_000);  // $86.00
const ETH_BASE_PRICE  = new BN(330_000_000_000); // $3300.00
const USDC_BASE_PRICE = new BN(100_000_000);     // $1.00

// ═══════════════════════════════════════════════════
// LOAD WALLET + PROGRAMS
// ═══════════════════════════════════════════════════

function loadKeypair(envVar: string, defaultPath: string): Keypair {
  const walletPath = process.env[envVar] ?? defaultPath;
  const resolved = walletPath.startsWith("~")
    ? path.join(os.homedir(), walletPath.slice(1))
    : walletPath;

  if (!fs.existsSync(resolved)) {
    throw new Error(
      `Wallet file not found: ${resolved}\n` +
      `Set ${envVar} env var or create the keypair with:\n` +
      `  solana-keygen new --outfile ${resolved}`
    );
  }
  const raw = JSON.parse(fs.readFileSync(resolved, "utf-8"));
  return Keypair.fromSecretKey(Uint8Array.from(raw));
}

function loadIdl(programName: string): object {
  const idlPath = path.join(
    __dirname, "..", "target", "idl", `${programName}.json`
  );
  if (!fs.existsSync(idlPath)) {
    throw new Error(
      `IDL not found: ${idlPath}\n` +
      `Run \`anchor build\` first to generate IDL files.`
    );
  }
  return JSON.parse(fs.readFileSync(idlPath, "utf-8"));
}

// ═══════════════════════════════════════════════════
// VAULT HELPER
// Create / find pool vault for a given mint.
// Pool PDA is the authority over all its vaults.
// ═══════════════════════════════════════════════════

async function getOrCreatePoolVault(
  connection: Connection,
  payer: Keypair,
  poolPda: PublicKey,
  mint: PublicKey,
): Promise<PublicKey> {
  const vaultAccount = await getOrCreateAssociatedTokenAccount(
    connection,
    payer,
    mint,
    poolPda,
    true, // allowOwnerOffCurve = true because poolPda is a PDA
  );
  console.log(`  vault (${mint.toBase58().slice(0, 8)}…): ${vaultAccount.address.toBase58()}`);
  return vaultAccount.address;
}

// ═══════════════════════════════════════════════════
// MAIN
// ═══════════════════════════════════════════════════

async function main() {
  // ── Provider setup ──────────────────────────────
  const rpcUrl = process.env.ANCHOR_PROVIDER_URL ?? "http://127.0.0.1:8899";
  const lpKeypair = loadKeypair(
    "ANCHOR_WALLET",
    path.join(os.homedir(), ".config/solana/id.json")
  );

  const connection = new Connection(rpcUrl, "confirmed");
  const wallet     = new anchor.Wallet(lpKeypair);
  const provider   = new AnchorProvider(connection, wallet, {
    commitment:         "confirmed",
    preflightCommitment: "confirmed",
  });
  anchor.setProvider(provider);

  console.log(`\nRPC       : ${rpcUrl}`);
  console.log(`Authority : ${lpKeypair.publicKey.toBase58()}`);

  // ── Load IDLs ────────────────────────────────────
  const poolIdl     = loadIdl("pool_program");
  const infoPoolIdl = loadIdl("info_pool_program");

  const POOL_PROGRAM_ID_     = new PublicKey("4AXtXF5VWeWKLqP6vHKPpjoc7wQ8r4duDqZ4CENtzsqZ");
  const INFO_POOL_PROGRAM_ID_ = new PublicKey("9MXoZpzQZzvURN1S1EARJLaDhFuGw3RAppQMYvGTcmPo");

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const poolProgram     = new Program(poolIdl as any, provider);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const infoPoolProgram = new Program(infoPoolIdl as any, provider);

  // ── Pre-compute Pool PDA ─────────────────────────
  const [poolPda] = findPoolPDA(lpKeypair.publicKey);
  console.log(`Pool PDA  : ${poolPda.toBase58()}`);

  // ── Create pool vaults ──────────────────────────
  // Each vault is an Associated Token Account owned by the Pool PDA
  console.log("\nCreating pool vaults...");
  const [solVault, ethVault, usdcVault] = await Promise.all([
    getOrCreatePoolVault(connection, lpKeypair, poolPda, SOL_MINT),
    getOrCreatePoolVault(connection, lpKeypair, poolPda, ETH_MINT),
    getOrCreatePoolVault(connection, lpKeypair, poolPda, USDC_MINT),
  ]);

  // ── User's own token accounts (for deposit) ──────
  console.log("\nFinding user token accounts...");
  const [userSolAta, userEthAta] = await Promise.all([
    getOrCreateAssociatedTokenAccount(connection, lpKeypair, SOL_MINT, lpKeypair.publicKey),
    getOrCreateAssociatedTokenAccount(connection, lpKeypair, ETH_MINT, lpKeypair.publicKey),
  ]);

  // ── Build config ─────────────────────────────────
  const config: PoolSetupConfig = {
    poolType:      "Public",
    baseAssetMint: SOL_MINT,

    assets: [
      // ── SOL ───────────────────────────────────────
      {
        mint:           SOL_MINT,
        vault:          solVault,
        maxPctMin:      15,           // must have at least 15%
        maxPctMax:      35,           // can hold up to 35%
        feeMin:         30,           // 0.30% — low fee when price trending up
        feeMax:         200,          // 2.00% — high fee when price crashing
        thresholdUp:    800,          // 8% up  → start IL protection
        thresholdDown:  400,          // 4% down → start IL protection
        initialBase:    SOL_BASE_PRICE,
        allowed:        [ETH_MINT, USDC_MINT], // SOL can swap with ETH & USDC
        depositAmount:  SOL_DEPOSIT_AMOUNT,
        userTokenAccount: userSolAta.address,
      },

      // ── ETH ───────────────────────────────────────
      {
        mint:           ETH_MINT,
        vault:          ethVault,
        maxPctMin:      10,
        maxPctMax:      30,
        feeMin:         30,
        feeMax:         250,          // ETH more volatile → higher max fee
        thresholdUp:    1000,         // 10% up
        thresholdDown:  500,          // 5%  down
        initialBase:    ETH_BASE_PRICE,
        allowed:        [SOL_MINT, USDC_MINT],
        depositAmount:  ETH_DEPOSIT_AMOUNT,
        userTokenAccount: userEthAta.address,
      },

      // ── USDC (stable) ─────────────────────────────
      {
        mint:           USDC_MINT,
        vault:          usdcVault,
        maxPctMin:      20,
        maxPctMax:      50,           // stable coin = higher % allowed
        feeMin:         5,            // 0.05% — near-zero fee for stable
        feeMax:         30,           // 0.30% max
        thresholdUp:    100,          // 1% — tight threshold for stable
        thresholdDown:  100,
        initialBase:    USDC_BASE_PRICE,
        allowed:        [SOL_MINT, ETH_MINT],
        depositAmount:  USDC_DEPOSIT_AMOUNT, // 0 = skip deposit
        isStable:       true,
        staticFeeBps:   5,
      },
    ],
  };

  // ── Run setup ────────────────────────────────────
  const client = new PoolSetupClient(
    poolProgram,
    infoPoolProgram,
    lpKeypair.publicKey,
    connection
  );

  const result = await client.setupPool(config);

  // ── Verify ───────────────────────────────────────
  await client.verifySetup(result.poolPda);

  // ── Print summary ────────────────────────────────
  console.log("\n═══════════════════════════════════════");
  console.log("  DONE — Pool is live and ready");
  console.log("═══════════════════════════════════════");
  console.log(`\n  Pool PDA     : ${result.poolPda.toBase58()}`);
  console.log(`  InfoPool PDA : ${result.infoPoolPda.toBase58()}`);
  console.log("\n  Asset PDAs:");
  for (const [mint, pda] of result.assetPdas) {
    console.log(`    ${mint.slice(0, 8)}…  →  ${pda.toBase58()}`);
  }
  console.log("\n  Transactions:");
  result.signatures.forEach((sig, i) => {
    const labels = ["init_pool", ...config.assets.map(a => `add_asset(${a.mint.toBase58().slice(0,6)})`), "init_info_pool", ...config.assets.filter(a => a.depositAmount?.gtn(0)).map(a => `deposit(${a.mint.toBase58().slice(0,6)})`)];
    console.log(`    ${i + 1}. [${labels[i] ?? "tx"}] ${sig}`);
  });
  console.log("");
  console.log("  Next steps:");
  console.log("  1. Start the crank:  ts-node scripts/crank.ts");
  console.log("");
}

main().catch((err) => {
  console.error("\n❌ Setup failed:");
  console.error(err.message ?? err);
  process.exit(1);
});
  
