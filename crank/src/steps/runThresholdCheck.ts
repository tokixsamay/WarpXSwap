import { PublicKey } from "@solana/web3.js";
import { Program } from "@coral-xyz/anchor";
import { logger } from "../logger";

/**
 * STEP 3 — run_threshold_check
 *
 * 3-layer evaluation for the asset:
 *   Layer 1 — TWAP: current > short > medium > long (or inverse for down)
 *   Layer 2 — Volume: volume_24h >= volume_prev × 1.10
 *   Layer 3 — Confidence: confidence < price × 2%
 *
 * When all 3 confirm: base shifts by min(growth, 100 bps) per cycle.
 * May CPI to Pool: block_inflow or unblock_inflow when threshold is
 * exceeded or recovered.
 */
export async function stepRunThresholdCheck(
  infoPoolProgram: Program,
  infoPoolPda:     PublicKey,
  poolProgramId:   PublicKey,
  poolAccountPda:  PublicKey,
  assetAccountPda: PublicKey,
  mint:            PublicKey,
  crankPubkey:     PublicKey,
): Promise<string | null> {
  const ctx = "runThresholdCheck";
  try {
    const tx = await infoPoolProgram.methods
      .runThresholdCheck(mint)
      .accounts({
        infoPool:    infoPoolPda,
        poolProgram: poolProgramId,
        poolAccount: poolAccountPda,
        assetAccount: assetAccountPda,
        crank:       crankPubkey,
      })
      .rpc({ commitment: "confirmed" });

    logger.debug(ctx, `OK mint=${mint.toBase58().slice(0, 8)} tx=${tx.slice(0, 16)}`);
    return tx;
  } catch (err) {
    logger.warn(ctx, `SKIP mint=${mint.toBase58().slice(0, 8)}`, err instanceof Error ? err.message : err);
    return null;
  }
}
