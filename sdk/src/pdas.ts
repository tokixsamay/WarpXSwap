import { PublicKey } from "@solana/web3.js";
import {
  POOL_PROGRAM_ID,
  INFO_POOL_PROGRAM_ID,
  ROUTING_PROGRAM_ID,
  POOL_SEED,
  ASSET_SEED,
  LP_DEPOSIT_SEED,
  INFO_POOL_SEED,
  ROUTER_SEED,
} from "./constants";

// ── Pool PDAs ───────────────────────────────────────────────

/**
 * Derive the PoolAccount PDA.
 * Seeds: [b"pool", authority]
 */
export function findPoolPda(authority: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [POOL_SEED, authority.toBuffer()],
    POOL_PROGRAM_ID,
  );
}

/**
 * Derive the AssetAccount PDA for a specific pool + mint.
 * Seeds: [b"asset", pool, mint]
 */
export function findAssetPda(pool: PublicKey, mint: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [ASSET_SEED, pool.toBuffer(), mint.toBuffer()],
    POOL_PROGRAM_ID,
  );
}

/**
 * Derive the LpDepositAccount PDA for (pool, mint, depositor).
 * Seeds: [b"lp_deposit", pool, mint, depositor]
 */
export function findLpDepositPda(
  pool: PublicKey,
  mint: PublicKey,
  depositor: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [LP_DEPOSIT_SEED, pool.toBuffer(), mint.toBuffer(), depositor.toBuffer()],
    POOL_PROGRAM_ID,
  );
}

// ── InfoPool PDAs ───────────────────────────────────────────

/**
 * Derive the InfoPoolAccount PDA for a pool.
 * Seeds: [b"info_pool", pool]
 */
export function findInfoPoolPda(pool: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [INFO_POOL_SEED, pool.toBuffer()],
    INFO_POOL_PROGRAM_ID,
  );
}

// ── Router PDA ──────────────────────────────────────────────

/**
 * Derive the RouterConfig PDA (singleton).
 * Seeds: [b"router"]
 */
export function findRouterPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [ROUTER_SEED],
    ROUTING_PROGRAM_ID,
  );
}
