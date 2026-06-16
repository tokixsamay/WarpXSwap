import { PublicKey } from "@solana/web3.js";
import { Program } from "@coral-xyz/anchor";
import { logger } from "../logger";

/**
 * STEP 2 — push_oracle_price_to_pool
 *
 * CPI from InfoPool to Pool: writes the freshly-read Pyth spot price into
 * AssetAccount.oracle_price so swap.rs uses the latest price without a
 * circular CPI dep (Pool → InfoPool → Pool).
 *
 * Must be called AFTER update_pyth_feeds for the same mint.
 */
export async function stepPushOraclePrice(
  infoPoolProgram: Program,
  infoPoolPda:     PublicKey,
  poolProgramId:   PublicKey,
  poolAccountPda:  PublicKey,
  assetAccountPda: PublicKey,
  mint:            PublicKey,
  crankPubkey:     PublicKey,
): Promise<string | null> {
  const ctx = "pushOraclePrice";
  try {
    const tx = await infoPoolProgram.methods
      .pushOraclePriceToPool(mint)
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
  
