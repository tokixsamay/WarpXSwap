import { PublicKey } from "@solana/web3.js";
import { Program, BN } from "@coral-xyz/anchor";
import { logger } from "../logger";

/**
 * STEP 5 (pool-level) — update_pool_metrics
 *
 * Bug #19 / Bug #21 fix: writes the Pool program's live `total_value`
 * (mapped to info_pool.pool_size) and `pool_weight` into InfoPool so
 * that the Routing program's `pool_is_active` guard passes.
 *
 * Without this step:
 *   - info_pool.pool_size  == 0 permanently → every pool filtered as inactive
 *   - info_pool.pool_weight == 1_000_000 permanently → stale routing tie-breaker
 *
 * The Pool account is read off-chain via the RPC connection embedded in
 * infoPoolProgram.provider.  Raw Borsh bytes are parsed at the known
 * Anchor field offsets (8-byte discriminator + struct fields in order):
 *
 *   off  0 –  7 : discriminator  (8 bytes)
 *   off  8      : pool_type      (u8, 1 byte)
 *   off  9 – 40 : owner          (Pubkey, 32 bytes)
 *   off 41 – 72 : base_asset     (Pubkey, 32 bytes)
 *   off 73 – 80 : total_value    (u64 LE, 8 bytes)  ← pool_size
 *   off 81 – 88 : pool_weight    (u64 LE, 8 bytes)
 *
 * Called once per pool (not per asset) after the per-asset 4-step cycle.
 */

const POOL_ACCOUNT_TOTAL_VALUE_OFFSET  = 8 + 1 + 32 + 32; // = 73
const POOL_ACCOUNT_POOL_WEIGHT_OFFSET  = POOL_ACCOUNT_TOTAL_VALUE_OFFSET + 8; // = 81

export async function stepUpdatePoolMetrics(
  infoPoolProgram: Program,
  infoPoolPda:     PublicKey,
  poolPda:         PublicKey,
  crankPubkey:     PublicKey,
): Promise<string | null> {
  const ctx = "updatePoolMetrics";
  try {
    const connection = (infoPoolProgram.provider as { connection: { getAccountInfo: (pubkey: PublicKey) => Promise<{ data: Buffer | Uint8Array } | null> } }).connection;
    const accountInfo = await connection.getAccountInfo(poolPda);

    if (!accountInfo) {
      logger.warn(ctx, `Pool account not found: ${poolPda.toBase58()}`);
      return null;
    }

    const buf        = Buffer.from(accountInfo.data);
    const poolSize   = buf.readBigUInt64LE(POOL_ACCOUNT_TOTAL_VALUE_OFFSET);
    const poolWeight = buf.readBigUInt64LE(POOL_ACCOUNT_POOL_WEIGHT_OFFSET);

    const tx = await infoPoolProgram.methods
      .updatePoolMetrics(
        new BN(poolSize.toString()),
        new BN(poolWeight.toString()),
      )
      .accounts({
        infoPool: infoPoolPda,
        crank:    crankPubkey,
      })
      .rpc({ commitment: "confirmed" });

    logger.debug(ctx, `OK pool=${poolPda.toBase58().slice(0, 8)} size=${poolSize} weight=${poolWeight} tx=${tx.slice(0, 16)}`);
    return tx;
  } catch (err) {
    logger.warn(ctx, `SKIP pool=${poolPda.toBase58().slice(0, 8)}`, err instanceof Error ? err.message : err);
    return null;
  }
      }
  
