import { PublicKey, Keypair } from "@solana/web3.js";
import { Program } from "@coral-xyz/anchor";
import { CrankConfig } from "./config";
import { logger } from "./logger";
import { stepUpdatePythFeeds }        from "./steps/updatePythFeeds";
import { stepPushOraclePrice }        from "./steps/pushOraclePrice";
import { stepRunThresholdCheck }      from "./steps/runThresholdCheck";
import { stepCalculateAndPushFee }    from "./steps/calculateAndPushFee";
import { stepPushVolume }             from "./steps/pushVolume";
import { stepUpdatePoolMetrics }      from "./steps/updatePoolMetrics";
import { findInfoPoolPda, findPoolPda, findAssetPda } from "../../sdk/src/pdas";

// ── Registered pool/asset config ─────────────────────────────

export interface AssetConfig {
  mint:            PublicKey;
  mintAddr:        string;         // base58 — for DexScreener lookup
  pythPriceAccount: PublicKey;     // Pyth V2 PriceUpdateV2 account
}

export interface PoolConfig {
  poolOwner:  PublicKey;           // authority used to derive Pool PDA
  assets:     AssetConfig[];
}

// ── Bug #22 fix: retry helper with exponential back-off ──────
//
// cfg.maxRetries and cfg.retryDelayMs were present in CrankConfig but
// never used — the crank had no retry logic at all.  A single transient
// RPC hiccup would silently skip the entire 4-step cycle for an asset.
//
// withRetry wraps a step function (returns string tx | null) and retries
// up to maxRetries times with doubling delay (500ms → 1s → 2s …).
// Steps indicate failure by returning null (they catch internally);
// the wrapper re-invokes until it gets a non-null result or exhausts retries.

async function withRetry(
  fn:           () => Promise<string | null>,
  maxRetries:   number,
  retryDelayMs: number,
  label:        string,
): Promise<string | null> {
  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    const result = await fn();
    if (result !== null) return result;

    if (attempt < maxRetries) {
      const backoffMs = retryDelayMs * Math.pow(2, attempt);
      logger.debug(
        "retry",
        `${label} attempt ${attempt + 1}/${maxRetries + 1} failed — retrying in ${backoffMs}ms`,
      );
      await delay(backoffMs);
    } else {
      logger.warn("retry", `${label} failed after ${maxRetries + 1} attempt(s)`);
    }
  }
  return null;
}

// ── Per-asset 4-step cycle ────────────────────────────────────

async function runAssetCycle(
  infoPoolProgram: Program,
  poolProgramId:   PublicKey,
  poolOwner:       PublicKey,
  asset:           AssetConfig,
  crankKeypair:    Keypair,
  cfg:             CrankConfig,
  onlyVolume:      boolean,
): Promise<void> {
  const [poolPda]      = findPoolPda(poolOwner);
  const [infoPoolPda]  = findInfoPoolPda(poolPda);
  const [assetPda]     = findAssetPda(poolPda, asset.mint);
  const crankPubkey    = crankKeypair.publicKey;
  const label          = `pool=${poolOwner.toBase58().slice(0, 8)} mint=${asset.mintAddr.slice(0, 8)}`;

  if (onlyVolume) {
    await withRetry(
      () => stepPushVolume(
        infoPoolProgram,
        infoPoolPda,
        asset.mint,
        asset.mintAddr,
        crankPubkey,
        cfg.dexscreenerBaseUrl,
      ),
      cfg.maxRetries,
      cfg.retryDelayMs,
      `pushVolume ${label}`,
    );
    return;
  }

  // STEP 1 — Read Pyth → update InfoPool EMAs + confidence
  await withRetry(
    () => stepUpdatePythFeeds(
      infoPoolProgram,
      infoPoolPda,
      poolPda,
      asset.mint,
      asset.pythPriceAccount,
      crankPubkey,
    ),
    cfg.maxRetries,
    cfg.retryDelayMs,
    `updatePythFeeds ${label}`,
  );

  // STEP 2 — Push oracle price to Pool's AssetAccount.oracle_price
  await withRetry(
    () => stepPushOraclePrice(
      infoPoolProgram,
      infoPoolPda,
      poolProgramId,
      poolPda,
      assetPda,
      asset.mint,
      crankPubkey,
    ),
    cfg.maxRetries,
    cfg.retryDelayMs,
    `pushOraclePrice ${label}`,
  );

  // STEP 3 — 3-layer threshold check (may block/unblock inflow)
  await withRetry(
    () => stepRunThresholdCheck(
      infoPoolProgram,
      infoPoolPda,
      poolProgramId,
      poolPda,
      assetPda,
      asset.mint,
      crankPubkey,
    ),
    cfg.maxRetries,
    cfg.retryDelayMs,
    `runThresholdCheck ${label}`,
  );

  // STEP 4 — Compute V-shape fee and push to Pool
  await withRetry(
    () => stepCalculateAndPushFee(
      infoPoolProgram,
      infoPoolPda,
      poolProgramId,
      poolPda,
      assetPda,
      asset.mint,
      crankPubkey,
    ),
    cfg.maxRetries,
    cfg.retryDelayMs,
    `calculateAndPushFee ${label}`,
  );
}

// ── Main loop ─────────────────────────────────────────────────

export async function startCrankLoop(
  infoPoolProgram: Program,
  poolProgramId:   PublicKey,
  pools:           PoolConfig[],
  cfg:             CrankConfig,
): Promise<void> {
  const crankKeypair   = cfg.crankKeypair;
  const slotMs         = cfg.slotIntervalMs;
  const volumeMs       = cfg.volumeIntervalMs;
  let   lastVolumePush = 0;

  logger.info("crank", `Starting — crank=${crankKeypair.publicKey.toBase58()}`);
  logger.info("crank", `Slot interval=${slotMs}ms  Volume interval=${volumeMs}ms`);
  logger.info("crank", `Retry: maxRetries=${cfg.maxRetries} retryDelayMs=${cfg.retryDelayMs}ms`);
  logger.info("crank", `Watching ${pools.length} pool(s)`);

  while (true) {
    const now      = Date.now();
    const doVolume = now - lastVolumePush >= volumeMs;
    if (doVolume) lastVolumePush = now;

    const slotStart = Date.now();

    for (const pool of pools) {
      const [poolPda]     = findPoolPda(pool.poolOwner);
      const [infoPoolPda] = findInfoPoolPda(poolPda);
      const crankPubkey   = crankKeypair.publicKey;

      // Run the 4-step per-asset cycle for every asset in this pool
      for (const asset of pool.assets) {
        try {
          await runAssetCycle(
            infoPoolProgram,
            poolProgramId,
            pool.poolOwner,
            asset,
            crankKeypair,
            cfg,
            false,
          );

          // Bug #11 fix: volume push fires asynchronously — do NOT await.
          //
          // The volume fetch involves an external HTTP call (DexScreener) that
          // can take hundreds of milliseconds or fail with a network timeout.
          // Awaiting it here blocks the entire slot cycle for all other assets,
          // causing the oracle/threshold/fee steps to fall behind and introducing
          // artificial oracle staleness.
          //
          // Instead, fire-and-forget with a .catch() so errors are logged but
          // never propagate into the slot loop.  Volume data being 1 cycle late
          // is acceptable — it is refreshed on every volumeIntervalMs tick and
          // the 3-period history requirement means a single missed cycle cannot
          // cause a false confirmation.
          if (doVolume) {
            runAssetCycle(
              infoPoolProgram,
              poolProgramId,
              pool.poolOwner,
              asset,
              crankKeypair,
              cfg,
              true,
            ).catch((err: unknown) => {
              logger.error(
                "crank",
                `Volume push failed pool=${pool.poolOwner.toBase58().slice(0, 8)} mint=${asset.mintAddr.slice(0, 8)}`,
                err,
              );
            });
          }
        } catch (err) {
          logger.error("crank", `Unhandled error pool=${pool.poolOwner.toBase58().slice(0,8)} mint=${asset.mintAddr.slice(0,8)}`, err);
        }
      }

      // STEP 5 (pool-level) — Bug #19/#21 fix: push Pool's live total_value
      // and pool_weight into InfoPool.pool_size / InfoPool.pool_weight.
      //
      // This must run once per pool AFTER all assets' 4-step cycles so
      // pool.total_value reflects the latest oracle-priced state.
      // Without this, info_pool.pool_size stays 0 and all pools are
      // permanently rejected by routing's `pool_is_active` guard.
      await withRetry(
        () => stepUpdatePoolMetrics(
          infoPoolProgram,
          infoPoolPda,
          poolPda,
          crankPubkey,
        ),
        cfg.maxRetries,
        cfg.retryDelayMs,
        `updatePoolMetrics pool=${pool.poolOwner.toBase58().slice(0, 8)}`,
      );
    }

    const elapsed = Date.now() - slotStart;
    const wait    = Math.max(0, slotMs - elapsed);
    if (wait > 0) await delay(wait);
  }
}

function delay(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
           }
      
