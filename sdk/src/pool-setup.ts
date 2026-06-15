import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import {
  getOrCreateAssociatedTokenAccount,
  createMint,
  mintTo,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { AnchorProvider, Program, BN, setProvider, web3 } from "@coral-xyz/anchor";
import { Wallet } from "@coral-xyz/anchor";
import {
  POOL_PROGRAM_ID,
  INFO_POOL_PROGRAM_ID,
  POOL_SEED,
  ASSET_SEED,
  INFO_POOL_SEED,
} from "./constants";
import { findPoolPDA, findAssetPDA, findInfoPoolPDA } from "./pda";

// ════════════════════════════════════════════════════════════════
// CONSTANTS
// ════════════════════════════════════════════════════════════════

/**
 * Minimum total USD value that must be deposited when creating a Private pool.
 * Enforced in setupPool() before any on-chain transaction is sent.
 */
export const PRIVATE_POOL_MIN_VALUE_USD = 100_000;

// ════════════════════════════════════════════════════════════════
// TYPES
// ════════════════════════════════════════════════════════════════

export interface AssetConfig {
  /** SPL token mint address */
  mint: PublicKey;
  /** Existing pool vault token account (must be owned by pool PDA) */
  vault: PublicKey;
  /** Min pool % (e.g. 10 = 10%) */
  maxPctMin: number;
  /** Max pool % (e.g. 35 = 35%) */
  maxPctMax: number;
  /** Min dynamic fee in basis points (e.g. 30 = 0.30%) */
  feeMin: number;
  /** Max dynamic fee in basis points (e.g. 200 = 2.00%) */
  feeMax: number;
  /** Upper IL threshold in basis points (e.g. 800 = 8%) */
  thresholdUp: number;
  /** Lower IL threshold in basis points (e.g. 400 = 4%) */
  thresholdDown: number;
  /** Starting reference price — Pyth-scale integer (price × 10^8) */
  initialBase: BN;
  /**
   * Number of decimals for this mint's native units.
   * Required when creating a Private pool so that initial deposit value
   * can be converted to USD and checked against PRIVATE_POOL_MIN_VALUE_USD.
   * (e.g. 9 for SOL/ETH, 6 for USDC, 8 for BTC)
   */
  mintDecimals?: number;
  /** Other mints this asset can be swapped with */
  allowed: PublicKey[];
  /** How many tokens to deposit as initial liquidity */
  depositAmount?: BN;
  /** User's token account to deposit from (required if depositAmount set) */
  userTokenAccount?: PublicKey;
  /** Mark as stablecoin — uses static fee instead of V-shape curve */
  isStable?: boolean;
  /** Static fee in basis points (required when isStable = true) */
  staticFeeBps?: number;
}

export interface PoolSetupConfig {
  /** Public or Private pool */
  poolType: "Public" | "Private";
  /** Base asset mint (e.g. BTC) */
  baseAssetMint: PublicKey;
  /** Assets to register in the pool */
  assets: AssetConfig[];
}

export interface PoolSetupResult {
  poolPda: PublicKey;
  infoPoolPda: PublicKey;
  assetPdas: Map<string, PublicKey>; // mint → AssetAccount PDA
  signatures: string[];
}

// ════════════════════════════════════════════════════════════════
// HELPERS
// ════════════════════════════════════════════════════════════════

/**
 * Computes the total USD value of all assets that have both a depositAmount
 * and a mintDecimals configured.
 *
 * USD value per asset = depositAmount / 10^mintDecimals × (initialBase / 10^8)
 *
 * @returns { totalUsd, breakdown } — breakdown lists each asset's contribution.
 */
export function computeDepositValueUsd(assets: AssetConfig[]): {
  totalUsd: number;
  breakdown: Array<{ mint: string; usd: number }>;
} {
  const breakdown: Array<{ mint: string; usd: number }> = [];
  let totalUsd = 0;

  for (const asset of assets) {
    if (!asset.depositAmount || asset.mintDecimals === undefined) continue;

    const priceUsd     = asset.initialBase.toNumber() / 1e8;
    const wholeTokens  = asset.depositAmount.toNumber() / Math.pow(10, asset.mintDecimals);
    const usd          = wholeTokens * priceUsd;

    breakdown.push({ mint: asset.mint.toBase58().slice(0, 8) + "…", usd });
    totalUsd += usd;
  }

  return { totalUsd, breakdown };
}

/**
 * Throws if the configured deposit for a Private pool is below
 * PRIVATE_POOL_MIN_VALUE_USD ($100,000).  Call before any on-chain tx.
 */
export function assertPrivatePoolMinValue(assets: AssetConfig[]): void {
  const { totalUsd, breakdown } = computeDepositValueUsd(assets);

  const missing = assets.filter(
    (a) => a.depositAmount && a.mintDecimals === undefined
  );
  if (missing.length > 0) {
    throw new Error(
      `Private pool requires mintDecimals on every asset with a depositAmount.\n` +
      `Missing mintDecimals for: ${missing.map((a) => a.mint.toBase58().slice(0, 8)).join(", ")}`
    );
  }

  const fmt = (n: number) =>
    new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", maximumFractionDigits: 0 }).format(n);

  console.log(
    `  Private pool deposit check: ${breakdown.map((b) => `${b.mint} ${fmt(b.usd)}`).join(" + ")} = ${fmt(totalUsd)}`
  );

  if (totalUsd < PRIVATE_POOL_MIN_VALUE_USD) {
    throw new Error(
      `Private pool creation rejected: initial deposit value ${fmt(totalUsd)} ` +
      `is below the required minimum of ${fmt(PRIVATE_POOL_MIN_VALUE_USD)}.\n` +
      `Increase depositAmount across assets until the total reaches $100,000 USD.`
    );
  }

  console.log(`  ✓ Minimum value check passed (${fmt(totalUsd)} ≥ ${fmt(PRIVATE_POOL_MIN_VALUE_USD)})`);
}

// ════════════════════════════════════════════════════════════════
// POOL SETUP CLIENT
// ════════════════════════════════════════════════════════════════

export class PoolSetupClient {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private poolProgram: any;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private infoPoolProgram: any;
  private authority: PublicKey;
  private connection: Connection;

  constructor(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    poolProgram: any,    // Program<PoolIDL>
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    infoPoolProgram: any, // Program<InfoPoolIDL>
    authority: PublicKey,
    connection: Connection
  ) {
    this.poolProgram     = poolProgram;
    this.infoPoolProgram = infoPoolProgram;
    this.authority       = authority;
    this.connection      = connection;
  }

  // ── STEP 1: Initialize Pool ─────────────────────────────────

  async initializePool(
    config: Pick<PoolSetupConfig, "poolType" | "baseAssetMint">
  ): Promise<{ poolPda: PublicKey; sig: string }> {
    const [poolPda] = findPoolPDA(this.authority);

    const poolType = config.poolType === "Public"
      ? { public: {} }
      : { private: {} };

    const sig = await this.poolProgram.methods
      .initializePool(poolType)
      .accounts({
        pool:           poolPda,
        baseAssetMint:  config.baseAssetMint,
        authority:      this.authority,
        systemProgram:  SystemProgram.programId,
      })
      .rpc();

    console.log(`  ✓ Pool initialized: ${poolPda.toBase58()}`);
    console.log(`    tx: ${sig}`);

    return { poolPda, sig };
  }

  // ── STEP 2: Add Asset ───────────────────────────────────────

  async addAsset(
    poolPda: PublicKey,
    asset: AssetConfig
  ): Promise<{ assetPda: PublicKey; sig: string }> {
    const [assetPda] = findAssetPDA(poolPda, asset.mint);

    const sig = await this.poolProgram.methods
      .addAsset({
        mint:          asset.mint,
        maxPctMin:     asset.maxPctMin,
        maxPctMax:     asset.maxPctMax,
        feeMin:        asset.feeMin,
        feeMax:        asset.feeMax,
        thresholdUp:   asset.thresholdUp,
        thresholdDown: asset.thresholdDown,
        initialBase:   asset.initialBase,
        allowed:       asset.allowed,
        isStable:      asset.isStable ?? false,
        staticFeeBps:  asset.staticFeeBps ?? 0,
      })
      .accounts({
        pool:          poolPda,
        asset:         assetPda,
        authority:     this.authority,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    console.log(`  ✓ Asset added: ${asset.mint.toBase58().slice(0, 8)}…`);
    console.log(`    AssetPDA: ${assetPda.toBase58()}`);
    console.log(`    fee: ${asset.feeMin}–${asset.feeMax} bps | threshold: ±${asset.thresholdUp}/${asset.thresholdDown} bps`);

    return { assetPda, sig };
  }

  // ── STEP 3: Initialize InfoPool ─────────────────────────────

  async initializeInfoPool(
    poolPda: PublicKey
  ): Promise<{ infoPoolPda: PublicKey; sig: string }> {
    const [infoPoolPda] = findInfoPoolPDA(poolPda);

    const sig = await this.infoPoolProgram.methods
      .initializeInfoPool(poolPda)
      .accounts({
        infoPool:      infoPoolPda,
        authority:     this.authority,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    console.log(`  ✓ InfoPool initialized: ${infoPoolPda.toBase58()}`);

    return { infoPoolPda, sig };
  }

  // ── STEP 4: Deposit Liquidity ───────────────────────────────

  async deposit(
    poolPda:          PublicKey,
    assetPda:         PublicKey,
    vault:            PublicKey,
    userTokenAccount: PublicKey,
    amount:           BN
  ): Promise<string> {
    const sig = await this.poolProgram.methods
      .deposit(amount)
      .accounts({
        pool:         poolPda,
        asset:        assetPda,
        poolVault:    vault,
        userToken:    userTokenAccount,
        user:         this.authority,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    console.log(`  ✓ Deposited ${amount.toString()} tokens`);
    console.log(`    vault: ${vault.toBase58()}`);

    return sig;
  }

  // ── FULL SETUP IN ONE CALL ──────────────────────────────────
  // Runs all 4 steps sequentially and returns final PDAs + signatures.

  async setupPool(config: PoolSetupConfig): Promise<PoolSetupResult> {
    const signatures: string[] = [];
    const assetPdas = new Map<string, PublicKey>();

    console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    console.log("  WarpXSwap — Pool Setup");
    console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    console.log(`  Authority : ${this.authority.toBase58()}`);
    console.log(`  Pool type : ${config.poolType}`);
    console.log(`  Assets    : ${config.assets.length}`);
    console.log("");

    // ── Pre-flight: Private pool $100k minimum ────────────────
    if (config.poolType === "Private") {
      assertPrivatePoolMinValue(config.assets);
    }

    // ── Step 1: Initialize Pool ──────────────────────────────
    console.log("Step 1 — Initialize Pool");
    const { poolPda, sig: sig1 } = await this.initializePool(config);
    signatures.push(sig1);

    // ── Step 2: Add all assets ────────────────────────────────
    console.log("\nStep 2 — Add Assets");
    for (const asset of config.assets) {
      const { assetPda, sig } = await this.addAsset(poolPda, asset);
      assetPdas.set(asset.mint.toBase58(), assetPda);
      signatures.push(sig);
    }

    // ── Step 3: Initialize InfoPool ──────────────────────────
    console.log("\nStep 3 — Initialize InfoPool");
    const { infoPoolPda, sig: sig3 } = await this.initializeInfoPool(poolPda);
    signatures.push(sig3);

    // ── Step 4: Deposit initial liquidity ────────────────────
    const toDeposit = config.assets.filter(
      (a) => a.depositAmount && a.userTokenAccount
    );
    if (toDeposit.length > 0) {
      console.log("\nStep 4 — Deposit Liquidity");
      for (const asset of toDeposit) {
        const assetPda = assetPdas.get(asset.mint.toBase58())!;
        const sig = await this.deposit(
          poolPda,
          assetPda,
          asset.vault,
          asset.userTokenAccount!,
          asset.depositAmount!
        );
        signatures.push(sig);
      }
    } else {
      console.log("\nStep 4 — Skipped (no depositAmount set)");
    }

    console.log("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    console.log("  Setup complete!");
    console.log(`  Pool PDA     : ${poolPda.toBase58()}`);
    console.log(`  InfoPool PDA : ${infoPoolPda.toBase58()}`);
    console.log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    return { poolPda, infoPoolPda, assetPdas, signatures };
  }

  // ── HELPER: Verify pool is fully set up ─────────────────────

  async verifySetup(poolPda: PublicKey): Promise<void> {
    console.log("\nVerifying setup...");

    const poolAccount = await this.poolProgram.account.poolAccount.fetch(poolPda);
    console.log(`  Pool active     : ${poolAccount.isActive}`);
    console.log(`  Pool type       : ${JSON.stringify(poolAccount.poolType)}`);
    console.log(`  Total value     : ${poolAccount.totalValue.toString()}`);

    const [infoPoolPda] = findInfoPoolPDA(poolPda);
    const infoPool = await this.infoPoolProgram.account.infoPoolAccount.fetch(infoPoolPda);
    console.log(`  InfoPool assets : ${infoPool.assets.length}`);
    console.log(`  Pool weight     : ${infoPool.poolWeight.toString()}`);
  }
}
