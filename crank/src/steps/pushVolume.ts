import { PublicKey } from "@solana/web3.js";
import { Program } from "@coral-xyz/anchor";
import { logger } from "../logger";

export interface VolumeData {
  mint:      PublicKey;
  mintAddr:  string;   // base58 mint address for API lookup
  volume24h: bigint;   // USD-scaled volume (e.g. from DexScreener)
}

/**
 * Fetch 24h volume from DexScreener for a given token mint address.
 * Returns 0n on any fetch failure — volume is non-critical (Layer 2).
 */
export async function fetchVolume(
  mintAddr:        string,
  dexscreenerBase: string,
): Promise<bigint> {
  try {
    const url      = `${dexscreenerBase}/tokens/${mintAddr}`;
    const response = await fetch(url, { signal: AbortSignal.timeout(5000) });
    if (!response.ok) return 0n;

    const data = await response.json() as {
      pairs?: Array<{ volume?: { h24?: number } }>;
    };
    const pairs = data.pairs ?? [];
    if (pairs.length === 0) return 0n;

    // Use the pair with the highest 24h volume (most liquid pair)
    const maxVolume = pairs.reduce((max, p) => {
      const v = p.volume?.h24 ?? 0;
      return v > max ? v : max;
    }, 0);

    // Scale to u64: multiply by 1e6 for precision (matches InfoPool's volume_24h unit)
    return BigInt(Math.round(maxVolume * 1_000_000));
  } catch {
    return 0n;
  }
}

/**
 * push_volume — Called at lower cadence (~60s) vs the 4-step per-slot loop.
 *
 * Pushes 24h trading volume into InfoPool for Layer 2 threshold check.
 * The on-chain program rotates: volume_prev ← volume_24h, then writes new value.
 * Volume Layer: confirms if volume_24h >= volume_prev × 1.10 (≥10% increase).
 */
export async function stepPushVolume(
  infoPoolProgram:  Program,
  infoPoolPda:      PublicKey,
  mint:             PublicKey,
  mintAddr:         string,
  crankPubkey:      PublicKey,
  dexscreenerBase:  string,
): Promise<void> {
  const ctx = "pushVolume";
  const volume24h = await fetchVolume(mintAddr, dexscreenerBase);

  if (volume24h === 0n) {
    logger.warn(ctx, `No volume data for mint=${mintAddr.slice(0, 8)} — skipping push`);
    return;
  }

  try {
    const tx = await infoPoolProgram.methods
      .pushVolume(mint, volume24h.toString())
      .accounts({
        infoPool: infoPoolPda,
        crank:    crankPubkey,
      })
      .rpc({ commitment: "confirmed" });

    logger.info(ctx, `OK mint=${mintAddr.slice(0, 8)} volume24h=${volume24h} tx=${tx.slice(0, 16)}`);
  } catch (err) {
    logger.warn(ctx, `FAIL mint=${mintAddr.slice(0, 8)}`, err instanceof Error ? err.message : err);
  }
      }
  
