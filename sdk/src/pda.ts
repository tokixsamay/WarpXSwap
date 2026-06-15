import { PublicKey } from "@solana/web3.js";
import BN from "bn.js";
import {
  GOVERNANCE_SEED,
  PROPOSAL_SEED,
  POOL_SEED,
  ASSET_SEED,
  INFO_POOL_SEED,
  GOVERNANCE_PROGRAM_ID,
  POOL_PROGRAM_ID,
  INFO_POOL_PROGRAM_ID,
} from "./constants";

// ── GOVERNANCE PDA ─────────────────────────────────────────────
// seeds: [b"governance", pool_id]
export function findGovernancePDA(
  poolId: PublicKey,
  programId: PublicKey = GOVERNANCE_PROGRAM_ID
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [GOVERNANCE_SEED, poolId.toBuffer()],
    programId
  );
}

// ── PROPOSAL PDA ───────────────────────────────────────────────
// seeds: [b"proposal", pool_id, proposal_id_le_bytes]
export function findProposalPDA(
  poolId: PublicKey,
  proposalId: BN,
  programId: PublicKey = GOVERNANCE_PROGRAM_ID
): [PublicKey, number] {
  const idBytes = proposalId.toArrayLike(Buffer, "le", 8);
  return PublicKey.findProgramAddressSync(
    [PROPOSAL_SEED, poolId.toBuffer(), idBytes],
    programId
  );
}

// ── POOL PDA ───────────────────────────────────────────────────
// seeds: [b"pool", owner]
export function findPoolPDA(
  owner: PublicKey,
  programId: PublicKey = POOL_PROGRAM_ID
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [POOL_SEED, owner.toBuffer()],
    programId
  );
}

// ── ASSET PDA ──────────────────────────────────────────────────
// seeds: [b"asset", pool, mint]
export function findAssetPDA(
  pool: PublicKey,
  mint: PublicKey,
  programId: PublicKey = POOL_PROGRAM_ID
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [ASSET_SEED, pool.toBuffer(), mint.toBuffer()],
    programId
  );
}

// ── INFO POOL PDA ──────────────────────────────────────────────
// seeds: [b"info_pool", pool_id]
export function findInfoPoolPDA(
  poolId: PublicKey,
  programId: PublicKey = INFO_POOL_PROGRAM_ID
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [INFO_POOL_SEED, poolId.toBuffer()],
    programId
  );
}

