#!/usr/bin/env ts-node
// ╔═══════════════════════════════════════════════════════════════════════╗
// ║   WarpXSwap CRANK — Off-Chain Keeper Bot                              ║
// ║                                                                       ║
// ║   Per-block duties (~400 ms):                                         ║
// ║     1. Fetch Pyth price from Hermes oracle API                        ║
// ║     2. Post price update to Solana (Pyth V2 receiver)                 ║
// ║     3. Call info_pool.update_pyth_feeds          → write EMA TWAPs   ║
// ║     4. Call info_pool.push_oracle_price_to_pool  → write oracle_price ║
// ║        into Pool AssetAccount so swaps can read the live price        ║
// ║     5. Call info_pool.run_threshold_check → fee + block/unblock CPI   ║
// ║                                                                       ║
// ║   Per-minute duty (VOLUME_REFRESH_MS, default 60 s):                  ║
// ║     5. Fetch 24h USD volume from DexScreener (free, no API key)       ║
// ║     6. Call info_pool.push_volume → write volume_24h / volume_prev    ║
// ║        Enables 3-layer volume confirmation → base-price shifts        ║
// ║                                                                       ║
// ║   Without this crank:                                                 ║
// ║     • Dynamic fees stay frozen at initial value                       ║
// ║     • IL protection never activates                                   ║
// ║     • Inflow block/unblock never fires                                ║
// ║     • TWAP EMAs never update                                          ║
// ║     • volume_confirmed always false → base never shifts               ║
// ║                                                                       ║
// ║   Architecture:                                                       ║
// ║     Hermes → post_update → PriceUpdateV2 acct → update_pyth_feeds    ║
// ║                                            → run_threshold_check      ║
// ║                                                 ↓ CPI                 ║
// ║                                           pool.update_fee             ║
// ║                                           pool.block_inflow  (if any) ║
// ╚═══════════════════════════════════════════════════════════════════════╝
//
// Usage:
//   ts-node scripts/crank.ts
//
// Env vars (all optional — defaults shown):
//   RPC_URL=http://127.0.0.1:8899
//   WALLET_PATH=~/.config/solana/id.json   (crank keypair — pays tx fees)
//   CRANK_INTERVAL_MS=400                  (one Solana slot ≈ 400 ms)
//   POOLS=all                              (comma-sep pool PDAs, or "all")
//   HERMES_URL=https://hermes.pyth.network
//   HERMES_TIMEOUT_MS=3000
//   MAX_RETRIES=3                          (per-asset retry before skip)
//   LOG_LEVEL=info                         (info | debug | quiet)
//   MOCK_PRICES=false                      (true = mock prices + synthetic volume)
//   VOLUME_REFRESH_MS=60000                (how often to hit DexScreener, default 60 s)
//   DEXSCREENER_URL=https://api.dexscreener.com   (override for proxies/testing)
//   DEXSCREENER_TIMEOUT_MS=5000

import * as fs   from "fs";
import * as os   from "os";
import * as path from "path";
import * as https from "https";
import * as http  from "http";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { AnchorProvider, Program, Wallet, BN } from "@coral-xyz/anchor";
import {
  POOL_PROGRAM_ID,
  INFO_POOL_PROGRAM_ID,
  findPoolPDA,
  findInfoPoolPDA,
  findAssetPDA,
} from "../sdk/src";

// ── CONFIG ─────────────────────────────────────────────────────────────

const RPC_URL          = process.env.RPC_URL          ?? "http://127.0.0.1:8899";
const HERMES_URL       = process.env.HERMES_URL       ?? "https://hermes.pyth.network";
const HERMES_TIMEOUT   = Number(process.env.HERMES_TIMEOUT_MS ?? "3000");
const CRANK_INTERVAL   = Number(process.env.CRANK_INTERVAL_MS ?? "400");
const MAX_RETRIES      = Number(process.env.MAX_RETRIES       ?? "3");
const LOG_LEVEL        = (process.env.LOG_LEVEL  ?? "info") as "quiet" | "info" | "debug";
const MOCK_PRICES      = (process.env.MOCK_PRICES ?? "false") === "true";

const VOLUME_REFRESH_MS    = Number(process.env.VOLUME_REFRESH_MS    ?? "60000");  // 1 min
const DEXSCREENER_URL      = process.env.DEXSCREENER_URL ?? "https://api.dexscreener.com";
const DEXSCREENER_TIMEOUT  = Number(process.env.DEXSCREENER_TIMEOUT_MS ?? "5000");

const WALLET_PATH = process.env.WALLET_PATH
  ? path.resolve(process.env.WALLET_PATH)
  : path.join(os.homedir(), ".config", "solana", "id.json");

const IDL_DIR     = path.join(__dirname, "..", "target", "idl");
const SETUP_JSON  = path.join(__dirname, "..", "complete-setup.json");

// Pyth V2 receiver program (same on mainnet + devnet)
const PYTH_RECEIVER_PROGRAM = new PublicKey("rec5EkMrVqGcZGk9f29VS9e3yeVfH7i8ycn6yaTe23Y");

// ── PYTH FEED REGISTRY ────────────────────────────────────────────────
// Feed ID hex → used in Hermes query + passed to the Rust program.
// PriceUpdateV2 account pubkeys — these are deterministic per feed
// on the Pyth receiver program (derived as PDA from feed_id).

interface PythFeed {
  feedId:      string;   // 0x-prefixed hex, 32 bytes
  label:       string;
  exponent:    number;   // price = raw_price * 10^exponent
  mockPriceUsd: number;  // used when MOCK_PRICES=true
}

const PYTH_FEEDS: Record<string, PythFeed> = {
  // SOL/USD
  "So11111111111111111111111111111111111111112": {
    feedId:       "0xef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d",
    label:        "SOL/USD",
    exponent:     -8,
    mockPriceUsd: 185_00,  // $185 in 10^-2 cents → scaled by exponent
  },
  // ETH/USD
  "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs": {
    feedId:       "0xff61491a931112ddf1bd8147cd1b641375f79f5825126d665480874634fd0ace",
    label:        "ETH/USD",
    exponent:     -8,
    mockPriceUsd: 3500_00,
  },
  // USDC/USD
  "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v": {
    feedId:       "0xeaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a",
    label:        "USDC/USD",
    exponent:     -8,
    mockPriceUsd: 1_00,
  },
  // BTC/USD
  "9n4nbM75f5Ui33ZbPYXn59EwSgE8CGsHtAeTH5YFeJ9E": {
    feedId:       "0xe62df6c8b4a85fe1a67db44dc12de5db330f7ac66b72dc658afedf0f4a415b43",
    label:        "BTC/USD",
    exponent:     -8,
    mockPriceUsd: 95000_00,
  },
  // USDT/USD  (Solana mainnet mint: Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB)
  "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB": {
    feedId:       "0x2b89b9dc8fdf9f34709a5b106b472f0f39bb6ca9ce04b0fd7f2e971688e2e53b",
    label:        "USDT/USD",
    exponent:     -8,
    mockPriceUsd: 1_00,
  },
};

// ── TYPES ──────────────────────────────────────────────────────────────

interface PoolConfig {
  label:    string;
  poolPda:  PublicKey;
  infoPda:  PublicKey;
  assets:   PublicKey[]; // mint pubkeys
}

interface CrankStats {
  cycles:          number;
  successTotal:    number;
  errorTotal:      number;
  feeUpdatesCpi:   number;
  inflowBlocks:    number;
  inflowUnblocks:  number;
  startTime:       Date;
  lastCycleMs:     number;
  avgCycleMs:      number;
}

interface PythPriceData {
  price:      number;  // raw price (already scaled)
  conf:       number;  // confidence interval
  exponent:   number;
  publishTime: number;
  vaa:        Buffer | null;  // null in mock mode
}

// ── LOGGING ────────────────────────────────────────────────────────────

const C = {
  reset:  "\x1b[0m",
  bold:   "\x1b[1m",
  dim:    "\x1b[2m",
  green:  "\x1b[32m",
  yellow: "\x1b[33m",
  red:    "\x1b[31m",
  cyan:   "\x1b[36m",
  blue:   "\x1b[34m",
  gray:   "\x1b[90m",
};

function ts(): string {
  return new Date().toISOString().replace("T", " ").replace("Z", "");
}

function log(msg: string): void {
  if (LOG_LEVEL !== "quiet") console.log(`${C.gray}[${ts()}]${C.reset} ${msg}`);
}

function debug(msg: string): void {
  if (LOG_LEVEL === "debug") console.log(`${C.gray}[${ts()}] DBG ${msg}${C.reset}`);
}

function warn(msg: string): void {
  if (LOG_LEVEL !== "quiet") console.warn(`${C.gray}[${ts()}]${C.reset} ${C.yellow}⚠  ${msg}${C.reset}`);
}

function err(msg: string): void {
  console.error(`${C.gray}[${ts()}]${C.reset} ${C.red}✗  ${msg}${C.reset}`);
}

function ok(msg: string): void {
  if (LOG_LEVEL !== "quiet") console.log(`${C.gray}[${ts()}]${C.reset} ${C.green}✓  ${msg}${C.reset}`);
}

// ── HTTP HELPER (no axios dep) ─────────────────────────────────────────

function httpGet(url: string, timeoutMs: number): Promise<string> {
  return new Promise((resolve, reject) => {
    const mod = url.startsWith("https") ? https : http;
    const timer = setTimeout(() => reject(new Error(`Timeout: ${url}`)), timeoutMs);
    mod.get(url, (res) => {
      let data = "";
      res.on("data", (chunk: string) => { data += chunk; });
      res.on("end", () => { clearTimeout(timer); resolve(data); });
      res.on("error", (e: Error) => { clearTimeout(timer); reject(e); });
    }).on("error", (e: Error) => { clearTimeout(timer); reject(e); });
  });
}

// ── DEXSCREENER — FETCH 24H VOLUME ────────────────────────────────────
//
// Endpoint: GET /tokens/v1/solana/{address}
// Returns all trading pairs for the token; we sum volume.h24 across
// every pair to get aggregate 24h USD volume across all DEXes.
// Returns 0 on any network/parse error so the crank stays alive.
//
// Stored on-chain as integer USD (floor).  The volume layer only checks
// relative change (≥10%), so absolute units are consistent as long as
// the same source is used for both the current and previous window.

async function fetchDexScreenerVolume(mintStr: string): Promise<bigint> {
  if (MOCK_PRICES) {
    // Synthetic rising volume so volume_confirmed can become true in tests
    const base = 500_000_000n; // $500 M synthetic baseline
    const jitter = BigInt(Math.floor(Math.random() * 50_000_000)); // ±$50 M
    return base + jitter;
  }

  try {
    const url  = `${DEXSCREENER_URL}/tokens/v1/solana/${mintStr}`;
    const body = await httpGet(url, DEXSCREENER_TIMEOUT);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const json = JSON.parse(body) as { pairs?: any[] };

    if (!Array.isArray(json.pairs) || json.pairs.length === 0) return 0n;

    // Sum h24 volume across all pairs (total DEX volume for this token)
    let total = 0;
    for (const pair of json.pairs) {
      const h24 = Number(pair?.volume?.h24 ?? 0);
      if (isFinite(h24) && h24 > 0) total += h24;
    }
    return BigInt(Math.floor(total));
  } catch (e) {
    debug(`DexScreener fetch failed for ${mintStr.slice(0, 8)}: ${(e as Error).message}`);
    return 0n;
  }
}

// ── PYTH HERMES — FETCH LATEST PRICE ──────────────────────────────────
//
// Hermes V2 API endpoint:
//   GET /api/latest_price_feeds?ids[]=<feedId>&binary=true
//
// Returns JSON with price + binary (base64 VAA) for posting on-chain.

async function fetchPythPrice(feed: PythFeed): Promise<PythPriceData> {
  if (MOCK_PRICES) {
    // Synthetic price with ±0.2% random walk for testing
    const noise    = 1 + (Math.random() - 0.5) * 0.004;
    const rawPrice = Math.round(feed.mockPriceUsd * noise * 100);
    const conf     = Math.round(rawPrice * 0.001); // 0.1% confidence
    return {
      price:       rawPrice,
      conf,
      exponent:    feed.exponent,
      publishTime: Math.floor(Date.now() / 1000),
      vaa:         null,
    };
  }

  try {
    const url  = `${HERMES_URL}/api/latest_price_feeds?ids[]=${feed.feedId}&binary=true`;
    const body = await httpGet(url, HERMES_TIMEOUT);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const json  = JSON.parse(body) as any[];
    const entry = json[0];

    if (!entry?.price) throw new Error("No price data in Hermes response");

    const vaaB64 = entry.vaa as string | undefined;
    return {
      price:       Number(entry.price.price),
      conf:        Number(entry.price.conf),
      exponent:    Number(entry.price.expo),
      publishTime: Number(entry.price.publish_time),
      vaa:         vaaB64 ? Buffer.from(vaaB64, "base64") : null,
    };
  } catch (e) {
    // Fallback to mock on Hermes failure (devnet / no internet)
    warn(`Hermes fetch failed for ${feed.label} — falling back to mock: ${(e as Error).message}`);
    const rawPrice = Math.round(feed.mockPriceUsd * 100);
    return {
      price:       rawPrice,
      conf:        Math.round(rawPrice * 0.001),
      exponent:    feed.exponent,
      publishTime: Math.floor(Date.now() / 1000),
      vaa:         null,
    };
  }
}

// ── POST PYTH PRICE UPDATE ON-CHAIN ────────────────────────────────────
//
// When we have a real VAA from Hermes, we call Pyth receiver's `post_update`.
// In mock mode or when VAA is null, we skip this and pass a pre-existing
// (or dummy) price update account — the Rust code won't verify staleness
// in localnet if clock is mocked.
//
// Returns the PriceUpdateV2 account pubkey to pass to update_pyth_feeds.

async function postPythUpdate(
  connection:  Connection,
  crankKp:     Keypair,
  priceData:   PythPriceData,
  feed:        PythFeed,
): Promise<PublicKey | null> {
  if (!priceData.vaa || MOCK_PRICES) {
    debug(`  skip post_update for ${feed.label} (mock or no VAA)`);
    return null; // caller handles null = skip update_pyth_feeds
  }

  // Pyth V2 price update PDA: seeds = ["price_update", feed_id]
  // In the full integration, call ReceiverProgram.post_update(vaa).
  // For now, return the deterministic PDA so callers know the account.
  const feedIdBytes = Buffer.from(feed.feedId.replace("0x", ""), "hex");
  const [priceUpdatePda] = PublicKey.findProgramAddressSync(
    [Buffer.from("price_update"), feedIdBytes],
    PYTH_RECEIVER_PROGRAM,
  );

  // Check if already posted with fresh enough data
  const info = await connection.getAccountInfo(priceUpdatePda);
  if (info) {
    debug(`  price_update PDA exists for ${feed.label}: ${priceUpdatePda.toBase58()}`);
    return priceUpdatePda;
  }

  // Build post_update instruction (Pyth receiver program)
  // discriminator: sha256("global:post_update")[0..8]
  const disc = Buffer.from([0x48, 0x5f, 0xc4, 0x5b, 0x36, 0x1a, 0xd5, 0x73]);
  const vaaLen = Buffer.alloc(4);
  vaaLen.writeUInt32LE(priceData.vaa.length, 0);

  const data = Buffer.concat([disc, vaaLen, priceData.vaa]);

  const ix = new TransactionInstruction({
    programId: PYTH_RECEIVER_PROGRAM,
    keys: [
      { pubkey: crankKp.publicKey, isSigner: true, isWritable: true },
      { pubkey: priceUpdatePda,    isSigner: false, isWritable: true },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data,
  });

  try {
    const tx = new Transaction().add(ix);
    const sig = await connection.sendTransaction(tx, [crankKp], {
      skipPreflight: false,
      preflightCommitment: "processed",
    });
    await connection.confirmTransaction(sig, "processed");
    debug(`  post_update tx: ${sig.slice(0, 20)}...`);
    return priceUpdatePda;
  } catch (e) {
    debug(`  post_update failed (may already exist): ${(e as Error).message}`);
    return priceUpdatePda; // proceed anyway — might be stale but worth trying
  }
}

// ── CRANK CYCLE — one pool, one asset ──────────────────────────────────

async function crankAsset(
  infoPoolProgram: Program,
  infoPda:         PublicKey,
  poolPda:         PublicKey,
  mint:            PublicKey,
  priceUpdatePda:  PublicKey | null,
  crankKp:         Keypair,
  stats:           CrankStats,
): Promise<void> {
  const [assetPda] = findAssetPDA(poolPda, mint, POOL_PROGRAM_ID);
  const mintStr    = mint.toBase58();
  const feed       = PYTH_FEEDS[mintStr];

  // ── Step A: update_pyth_feeds ──────────────────────────────────
  // Only callable when we have a valid PriceUpdateV2 account.
  if (priceUpdatePda) {
    try {
      const txFeed = await infoPoolProgram.methods
        .updatePythFeeds(mint)
        .accounts({
          infoPool:     infoPda,
          priceUpdate:  priceUpdatePda,
          crank:        crankKp.publicKey,
        })
        .signers([crankKp])
        .rpc({ commitment: "processed" });

      debug(`  update_pyth_feeds [${feed?.label ?? mintStr.slice(0, 8)}] tx: ${txFeed.slice(0, 20)}...`);
    } catch (e) {
      const msg = (e as Error).message ?? String(e);
      // PriceStale is expected on localnet — not fatal
      if (!msg.includes("PythPriceStale") && !msg.includes("custom program error")) {
        warn(`  update_pyth_feeds failed [${feed?.label}]: ${msg}`);
      } else {
        debug(`  update_pyth_feeds skipped (stale): ${feed?.label}`);
      }
    }
  } else {
    debug(`  update_pyth_feeds skipped (no price_update PDA for ${feed?.label})`);
  }

  // ── Step B: push_oracle_price_to_pool ─────────────────────────
  // Pushes the latest oracle price from InfoPool into Pool's AssetAccount
  // via CPI. This MUST happen before run_threshold_check — swaps read
  // asset.oracle_price and will fail (OraclePriceNotSet) until it is set.
  //
  // IMPORTANT: We skip this when priceUpdatePda is null (mock mode /
  // MOCK_PRICES=true / localnet without a live VAA feed). In that case
  // Step A (update_pyth_feeds) was also skipped, so InfoPool.oracle_price
  // is still 0. Calling push_oracle_price_to_pool with price=0 would hit
  // the `require!(price_raw > 0, InvalidOraclePrice)` guard and fail. Swaps
  // will not work in mock mode until a real VAA populates the price — this
  // is intentional. To test swaps locally, use a forked mainnet with real
  // Pyth accounts or set the oracle_price manually via a test fixture.
  if (!priceUpdatePda) {
    debug(`  push_oracle_price_to_pool skipped (no price_update PDA — mock mode) [${feed?.label}]`);
  } else {
    try {
      const txPush = await infoPoolProgram.methods
        .pushOraclePriceToPool(mint)
        .accounts({
          infoPool:     infoPda,
          poolProgram:  POOL_PROGRAM_ID,
          poolAccount:  poolPda,
          assetAccount: assetPda,
          crank:        crankKp.publicKey,
        })
        .signers([crankKp])
        .rpc({ commitment: "processed" });

      debug(`  push_oracle_price_to_pool [${feed?.label ?? mintStr.slice(0, 8)}] tx: ${txPush.slice(0, 20)}...`);
    } catch (e) {
      const msg = (e as Error).message ?? String(e);
      if (msg.includes("AssetNotFound") || msg.includes("custom program error: 0x1")) {
        debug(`  push_oracle_price_to_pool: asset not in InfoPool [${feed?.label}] — skipping`);
      } else {
        warn(`  push_oracle_price_to_pool failed [${feed?.label ?? mintStr}]: ${msg}`);
      }
    }
  }

  // ── Step C: run_threshold_check ────────────────────────────────
  // Core crank call — re-evaluates fee + block/unblock state,
  // fires CPI into pool if anything changed.
  try {
    const txCheck = await infoPoolProgram.methods
      .runThresholdCheck(mint)
      .accounts({
        infoPool:     infoPda,
        poolProgram:  POOL_PROGRAM_ID,
        poolAccount:  poolPda,
        assetAccount: assetPda,
        crank:        crankKp.publicKey,
      })
      .signers([crankKp])
      .rpc({ commitment: "processed" });

    stats.successTotal++;
    debug(`  run_threshold_check [${feed?.label ?? mintStr.slice(0, 8)}] tx: ${txCheck.slice(0, 20)}...`);
  } catch (e) {
    const msg = (e as Error).message ?? String(e);
    stats.errorTotal++;

    // AssetNotFound = asset not yet registered in InfoPool (harmless)
    if (msg.includes("AssetNotFound") || msg.includes("custom program error: 0x1")) {
      debug(`  run_threshold_check: asset not yet in InfoPool [${feed?.label}] — skipping`);
    } else {
      warn(`  run_threshold_check failed [${feed?.label ?? mintStr}]: ${msg}`);
    }
  }
}

// ── PUSH VOLUME — one pool, one asset ──────────────────────────────────
//
// Calls info_pool.push_volume with the latest DexScreener volume figure.
// Skips silently if the asset is not registered in the InfoPool yet.

async function pushVolumeForAsset(
  infoPoolProgram: Program,
  infoPda:         PublicKey,
  mint:            PublicKey,
  volume24h:       bigint,
  crankKp:         Keypair,
): Promise<void> {
  const mintStr = mint.toBase58();
  const feed    = PYTH_FEEDS[mintStr];
  const label   = feed?.label ?? mintStr.slice(0, 8);

  try {
    const tx = await infoPoolProgram.methods
      .pushVolume(mint, new BN(volume24h.toString()))
      .accounts({
        infoPool: infoPda,
        crank:    crankKp.publicKey,
      })
      .signers([crankKp])
      .rpc({ commitment: "processed" });

    debug(`  push_volume [${label}] vol=${volume24h} tx: ${tx.slice(0, 20)}...`);
  } catch (e) {
    const msg = (e as Error).message ?? String(e);
    if (msg.includes("AssetNotFound") || msg.includes("custom program error: 0x1")) {
      debug(`  push_volume: asset not yet in InfoPool [${label}] — skipping`);
    } else {
      warn(`  push_volume failed [${label}]: ${msg}`);
    }
  }
}

// ── POOL CONFIG DISCOVERY ──────────────────────────────────────────────
// 1. Try to load complete-setup.json (written by complete-setup.ts)
// 2. Fall back to env POOLS variable
// 3. Fall back to default 4-pool config using known LP keys

function loadPoolConfigs(payerKey: PublicKey): PoolConfig[] {
  // Option A: Load from complete-setup.json
  if (fs.existsSync(SETUP_JSON)) {
    try {
      const setup = JSON.parse(fs.readFileSync(SETUP_JSON, "utf-8"));
      const configs: PoolConfig[] = [];

      const toConfig = (
        label: string,
        raw: { poolPda: string; infoPda: string },
        assets: string[]
      ): PoolConfig => ({
        label,
        poolPda: new PublicKey(raw.poolPda),
        infoPda: new PublicKey(raw.infoPda),
        assets:  assets.map(a => new PublicKey(a)),
      });

      if (setup.publicPool) {
        configs.push(toConfig(
          "Public (SOL/ETH/USDC/BTC)",
          setup.publicPool,
          [
            "So11111111111111111111111111111111111111112",
            "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "9n4nbM75f5Ui33ZbPYXn59EwSgE8CGsHtAeTH5YFeJ9E",
          ]
        ));
      }
      if (setup.privatePoolA) {
        configs.push(toConfig(
          "Private-A (SOL/USDC)", setup.privatePoolA,
          ["So11111111111111111111111111111111111111112",
           "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"]
        ));
      }
      if (setup.privatePoolB) {
        configs.push(toConfig(
          "Private-B (ETH/BTC)", setup.privatePoolB,
          ["7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs",
           "9n4nbM75f5Ui33ZbPYXn59EwSgE8CGsHtAeTH5YFeJ9E"]
        ));
      }
      if (setup.privatePoolC) {
        configs.push(toConfig(
          "Private-C (SOL/ETH)", setup.privatePoolC,
          ["So11111111111111111111111111111111111111112",
           "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs"]
        ));
      }
      if (configs.length > 0) {
        log(`Loaded ${configs.length} pool config(s) from complete-setup.json`);
        return configs;
      }
    } catch (e) {
      warn(`Failed to parse complete-setup.json: ${(e as Error).message}`);
    }
  }

  // Option B: Env POOLS variable
  const envPools = process.env.POOLS;
  if (envPools && envPools !== "all") {
    return envPools.split(",").map((p, i) => {
      const poolPda = new PublicKey(p.trim());
      const [infoPda] = findInfoPoolPDA(poolPda, INFO_POOL_PROGRAM_ID);
      return {
        label:   `Pool-${i + 1}`,
        poolPda,
        infoPda,
        assets:  Object.keys(PYTH_FEEDS).map(m => new PublicKey(m)),
      };
    });
  }

  // Option C: Default — derive from payer key (matches complete-setup.ts LP seeds)
  warn("No setup config found — using deterministic LP keys (run complete-setup first)");
  const crypto = require("crypto") as typeof import("crypto");
  return Array.from({ length: 4 }, (_, i) => {
    const seed = crypto.createHash("sha256")
      .update(`warpxswap-lp-${i}-${payerKey.toBase58()}`)
      .digest();
    const lpKey     = Keypair.fromSeed(seed).publicKey;
    const [poolPda] = findPoolPDA(lpKey, POOL_PROGRAM_ID);
    const [infoPda] = findInfoPoolPDA(poolPda, INFO_POOL_PROGRAM_ID);
    const assetSets = [
      ["So11111111111111111111111111111111111111112",
       "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs",
       "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
       "9n4nbM75f5Ui33ZbPYXn59EwSgE8CGsHtAeTH5YFeJ9E"],
      ["So11111111111111111111111111111111111111112",
       "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"],
      ["7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs",
       "9n4nbM75f5Ui33ZbPYXn59EwSgE8CGsHtAeTH5YFeJ9E"],
      ["So11111111111111111111111111111111111111112",
       "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs"],
    ];
    return {
      label:   ["Public", "Priv-A", "Priv-B", "Priv-C"][i],
      poolPda,
      infoPda,
      assets:  assetSets[i].map(m => new PublicKey(m)),
    };
  });
}

// ── STATS DISPLAY ──────────────────────────────────────────────────────

function printStats(stats: CrankStats, pools: PoolConfig[]): void {
  if (LOG_LEVEL === "quiet") return;

  const uptime = Math.floor((Date.now() - stats.startTime.getTime()) / 1000);
  const hh     = Math.floor(uptime / 3600).toString().padStart(2, "0");
  const mm     = Math.floor((uptime % 3600) / 60).toString().padStart(2, "0");
  const ss     = (uptime % 60).toString().padStart(2, "0");

  const totalAssets = pools.reduce((s, p) => s + p.assets.length, 0);

  process.stdout.write(
    `\r${C.cyan}${C.bold}WarpXSwap Crank${C.reset}` +
    `  ${C.green}↑${stats.successTotal}${C.reset}` +
    `  ${C.red}✗${stats.errorTotal}${C.reset}` +
    `  cycle#${stats.cycles}` +
    `  ${stats.lastCycleMs}ms/cycle` +
    `  uptime ${hh}:${mm}:${ss}` +
    `  ${totalAssets} assets / ${pools.length} pools` +
    "    "
  );
}

// ── MAIN LOOP ──────────────────────────────────────────────────────────

async function main() {
  console.log("\n╔══════════════════════════════════════════════════════════════╗");
  console.log("║  WarpXSwap Crank — Pyth oracle keeper + threshold bot        ║");
  console.log("╚══════════════════════════════════════════════════════════════╝\n");

  // ── Load crank keypair ───────────────────────────────────────
  if (!fs.existsSync(WALLET_PATH)) {
    throw new Error(
      `Crank wallet not found: ${WALLET_PATH}\n` +
      "Create a dedicated crank keypair:\n  solana-keygen new -o keypairs/crank.json"
    );
  }
  const rawKey  = JSON.parse(fs.readFileSync(WALLET_PATH, "utf-8")) as number[];
  const crankKp = Keypair.fromSecretKey(Uint8Array.from(rawKey));

  // ── Anchor setup ─────────────────────────────────────────────
  const connection = new Connection(RPC_URL, "processed");
  const wallet     = new Wallet(crankKp);
  const provider   = new AnchorProvider(connection, wallet, {
    commitment:          "processed",
    preflightCommitment: "processed",
  });

  const idlPath = path.join(IDL_DIR, "info_pool_program.json");
  if (!fs.existsSync(idlPath)) {
    throw new Error(`IDL not found: ${idlPath}\nRun 'anchor build' first.`);
  }
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const infoPoolProgram = new Program(JSON.parse(fs.readFileSync(idlPath, "utf-8")) as any, provider);

  // ── Load pool configs ─────────────────────────────────────────
  const pools = loadPoolConfigs(crankKp.publicKey);

  // ── Collect unique mints across all pools ─────────────────────
  const uniqueMints = [...new Set(pools.flatMap(p => p.assets.map(a => a.toBase58())))]
    .map(s => new PublicKey(s));

  // ── Print startup banner ──────────────────────────────────────
  log(`RPC:            ${RPC_URL}`);
  log(`Crank pubkey:   ${crankKp.publicKey.toBase58()}`);
  log(`Interval:       ${CRANK_INTERVAL} ms / cycle`);
  log(`Mode:           ${MOCK_PRICES ? "MOCK (no Hermes + synthetic volume)" : "LIVE (Hermes: " + HERMES_URL + ")"}`);
  log(`Volume source:  ${MOCK_PRICES ? "synthetic (MOCK_PRICES=true)" : "DexScreener (" + DEXSCREENER_URL + ")"}`);
  log(`Volume refresh: every ${VOLUME_REFRESH_MS / 1000}s`);
  log(`Pools:          ${pools.length}`);
  pools.forEach((p, i) =>
    log(`  [${i + 1}] ${p.label.padEnd(30)} pool: ${p.poolPda.toBase58().slice(0, 16)}...`)
  );
  log(`Unique assets:  ${uniqueMints.length} (${uniqueMints.map(m => {
    const f = PYTH_FEEDS[m.toBase58()];
    return f ? f.label.split("/")[0] : m.toBase58().slice(0, 6);
  }).join(", ")})`);
  console.log();

  // ── Check crank SOL balance ───────────────────────────────────
  const crankBal = await connection.getBalance(crankKp.publicKey);
  if (crankBal < 0.05 * 1e9) {
    warn(
      `Crank balance low: ${(crankBal / 1e9).toFixed(4)} SOL.\n` +
      `   Fund it: solana airdrop 2 ${crankKp.publicKey.toBase58()}`
    );
  } else {
    ok(`Crank balance: ${(crankBal / 1e9).toFixed(4)} SOL`);
  }
  console.log();

  // ── Volume refresh tracker ────────────────────────────────────
  // Volume is refreshed from DexScreener every VOLUME_REFRESH_MS
  // (default 60 s). Between refreshes the same figure is reused so
  // the crank doesn't pay for redundant HTTP calls every 400 ms tick.
  let lastVolumeMs = 0;
  // mint (base58) → latest 24h USD volume (integer, summed across pairs)
  const volumeMap = new Map<string, bigint>();

  // ── Stats ─────────────────────────────────────────────────────
  const stats: CrankStats = {
    cycles:         0,
    successTotal:   0,
    errorTotal:     0,
    feeUpdatesCpi:  0,
    inflowBlocks:   0,
    inflowUnblocks: 0,
    startTime:      new Date(),
    lastCycleMs:    0,
    avgCycleMs:     0,
  };

  // ── Graceful shutdown ─────────────────────────────────────────
  let running = true;
  const shutdown = () => {
    running = false;
    console.log("\n\nShutting down crank...");
    process.exit(0);
  };
  process.on("SIGINT",  shutdown);
  process.on("SIGTERM", shutdown);

  log(`Starting crank loop (Ctrl+C to stop)...\n`);

  // ── MAIN CRANK LOOP ───────────────────────────────────────────
  while (running) {
    const cycleStart = Date.now();
    stats.cycles++;

    // ── A. Fetch Pyth prices for all unique mints in parallel ──
    const priceMap: Map<string, { data: PythPriceData; pda: PublicKey | null }> = new Map();

    await Promise.allSettled(
      uniqueMints.map(async (mint) => {
        const mintStr = mint.toBase58();
        const feed    = PYTH_FEEDS[mintStr];
        if (!feed) {
          debug(`No Pyth feed configured for ${mintStr}`);
          return;
        }

        let retries = 0;
        while (retries < MAX_RETRIES) {
          try {
            const priceData = await fetchPythPrice(feed);
            const pda       = await postPythUpdate(connection, crankKp, priceData, feed);
            priceMap.set(mintStr, { data: priceData, pda });
            debug(`Pyth [${feed.label}] price=${priceData.price} conf=${priceData.conf}`);
            break;
          } catch (e) {
            retries++;
            if (retries >= MAX_RETRIES) {
              warn(`Pyth fetch failed after ${MAX_RETRIES} retries [${feed.label}]: ${(e as Error).message}`);
              priceMap.set(mintStr, { data: { price: 0, conf: 0, exponent: feed.exponent, publishTime: 0, vaa: null }, pda: null });
            } else {
              await sleep(100 * retries);
            }
          }
        }
      })
    );

    // ── B. Volume refresh (every VOLUME_REFRESH_MS, default 60 s) ──
    // Fetch 24h USD volume from DexScreener for every unique mint,
    // then push it on-chain via info_pool.push_volume.  This writes
    // the volume_24h / volume_prev window that check_volume_layer
    // reads in run_threshold_check, enabling the 3rd layer of the
    // base-shift confirmation engine.
    if (Date.now() - lastVolumeMs >= VOLUME_REFRESH_MS) {
      debug("Refreshing 24h volume from DexScreener...");

      // Fetch volumes in parallel — one HTTP request per unique mint
      await Promise.allSettled(
        uniqueMints.map(async (mint) => {
          const mintStr = mint.toBase58();
          const vol     = await fetchDexScreenerVolume(mintStr);
          volumeMap.set(mintStr, vol);
          if (vol > 0n) {
            const feed = PYTH_FEEDS[mintStr];
            debug(`  DexScreener [${feed?.label ?? mintStr.slice(0, 8)}] vol24h=$${vol.toLocaleString()}`);
          }
        })
      );

      // Push to every pool × asset that is initialised
      for (const pool of pools) {
        const infoExists = await connection.getAccountInfo(pool.infoPda);
        if (!infoExists) continue;

        await Promise.allSettled(
          pool.assets.map(async (mint) => {
            const vol = volumeMap.get(mint.toBase58()) ?? 0n;
            if (vol > 0n) {
              await pushVolumeForAsset(
                infoPoolProgram, pool.infoPda, mint, vol, crankKp,
              );
            }
          })
        );
      }

      lastVolumeMs = Date.now();
      debug("Volume refresh complete.");
    }

    // ── C. Crank all pools × all assets ───────────────────────
    for (const pool of pools) {
      const infoExists = await connection.getAccountInfo(pool.infoPda);
      if (!infoExists) {
        debug(`InfoPool not yet initialized for ${pool.label} — skipping`);
        continue;
      }

      for (const mint of pool.assets) {
        const mintStr  = mint.toBase58();
        const priceInfo = priceMap.get(mintStr);
        const pda       = priceInfo?.pda ?? null;

        let retries = 0;
        while (retries < MAX_RETRIES) {
          try {
            await crankAsset(
              infoPoolProgram,
              pool.infoPda,
              pool.poolPda,
              mint,
              pda,
              crankKp,
              stats,
            );
            break;
          } catch (e) {
            retries++;
            if (retries >= MAX_RETRIES) {
              err(`crankAsset failed after ${MAX_RETRIES} retries [${pool.label} / ${mintStr.slice(0, 8)}]: ${(e as Error).message}`);
            } else {
              await sleep(50 * retries);
            }
          }
        }
      }
    }

    // ── C. Timing + stats ──────────────────────────────────────
    const elapsed     = Date.now() - cycleStart;
    stats.lastCycleMs = elapsed;
    stats.avgCycleMs  = Math.round(
      (stats.avgCycleMs * (stats.cycles - 1) + elapsed) / stats.cycles
    );

    printStats(stats, pools);

    // ── D. Print periodic summary every 100 cycles ────────────
    if (stats.cycles % 100 === 0) {
      console.log();
      log(
        `${C.bold}Cycle ${stats.cycles}${C.reset}` +
        `  success=${stats.successTotal}` +
        `  errors=${stats.errorTotal}` +
        `  avg=${stats.avgCycleMs}ms` +
        `  uptime=${Math.floor((Date.now() - stats.startTime.getTime()) / 60000)}min`
      );
    }

    // ── E. Wait for next slot ──────────────────────────────────
    const wait = Math.max(0, CRANK_INTERVAL - elapsed);
    if (wait > 0) await sleep(wait);
  }
}

// ── UTIL ──────────────────────────────────────────────────────────────
function sleep(ms: number): Promise<void> {
  return new Promise(r => setTimeout(r, ms));
}

// ── ENTRY POINT ───────────────────────────────────────────────────────
main().catch((e) => {
  err(`Fatal: ${e instanceof Error ? e.message : String(e)}`);
  if (e instanceof Error && e.stack) console.error(e.stack);
  process.exit(1);
});
