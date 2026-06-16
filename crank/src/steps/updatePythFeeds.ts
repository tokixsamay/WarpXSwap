import { PublicKey, TransactionInstruction } from "@solana/web3.js";
import { Program } from "@coral-xyz/anchor";
import { logger } from "../logger";

/**
 * STEP 1 — update_pyth_feeds
 *
 * Reads the Pyth V2 PriceUpdateV2 account for the asset mint and updates
 * the InfoPool's EMA accumulators (twap_short, twap_medium, twap_long),
 * confidence interval, and current spot price.
 *
 * Crank authority constraint is enforced on-chain.
 */
export async function stepUpdatePythFeeds(
  program:         Program,
  infoPoolPda:     PublicKey,
  poolAccountPda:  PublicKey,
  mint:            PublicKey,
  pythPriceAccount: PublicKey,
  crankPubkey:     PublicKey,
): Promise<string | null> {
  const ctx = "updatePythFeeds";
  try {
    const tx = await program.methods
      .updatePythFeeds(mint)
      .accounts({
        infoPool:         infoPoolPda,
        pythPriceAccount,
        crank:            crankPubkey,
      })
      .rpc({ commitment: "confirmed" });

    logger.debug(ctx, `OK mint=${mint.toBase58().slice(0, 8)} tx=${tx.slice(0, 16)}`);
    return tx;
  } catch (err) {
    logger.warn(ctx, `SKIP mint=${mint.toBase58().slice(0, 8)}`, err instanceof Error ? err.message : err);
    return null;
  }
        }
