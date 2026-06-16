import { PublicKey } from "@solana/web3.js";
import { Program } from "@coral-xyz/anchor";
import { logger } from "../logger";

/**
 * STEP 4 — calculate_and_push_fee
 *
 * Computes the new fee for the asset and pushes it to Pool via CPI.
 * For volatile assets: V-shape curve based on distance from current_base.
 * For stable assets (is_stable = true): static_fee_bps used directly.
 *
 * Fee is only pushed if it changed (gas optimisation on-chain).
 *
 * Crank authority constraint enforced on-chain (bug fix applied —
 * previously any signer could call this instruction).
 */
export async function stepCalculateAndPushFee(
  infoPoolProgram: Program,
  infoPoolPda:     PublicKey,
  poolProgramId:   PublicKey,
  poolAccountPda:  PublicKey,
  assetAccountPda: PublicKey,
  mint:            PublicKey,
  crankPubkey:     PublicKey,
): Promise<string | null> {
  const ctx = "calculateAndPushFee";
  try {
    const tx = await infoPoolProgram.methods
      .calculateAndPushFee(mint)
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
  
