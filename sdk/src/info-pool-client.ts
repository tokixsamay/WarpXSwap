import { PublicKey } from "@solana/web3.js";
import { BN } from "@coral-xyz/anchor";
import { findInfoPoolPDA, findAssetPDA, findPoolPDA } from "./pda";
import { INFO_POOL_PROGRAM_ID, POOL_PROGRAM_ID } from "./constants";

// ═══════════════════════════════════════════════════════════════════
// INFO POOL CLIENT
//
// Wraps InfoPool program: read state, crank calls, governance updates.
//
// The InfoPool is the Pyth oracle + 3-layer IL engine for a pool.
// It stores per-asset: TWAP, price confidence, volume, fee, threshold.
//
// Usage:
//   const { infoPool } = createPrograms(provider);
//   const client = new InfoPoolClient(infoPool, poolOwner);
//
//   // Read current fee for SOL
//   const fee = await client.getAssetFee(solMint);
//   console.log(`SOL fee: ${fee / 100}%`);
//
//   // Read threshold state
//   const state = await client.getThresholdState(solMint);
//   console.log(`SOL blocked: ${state.isBlocked}`);
// ═══════════════════════════════════════════════════════════════════

export interface AssetFeeInfo {
  feeBps:      number;
  feeMinBps:   number;
  feeMaxBps:   number;
}

export interface ThresholdInfo {
  isBlocked:     boolean;
  thresholdUp:   number;   // bps (e.g. 500 = 5%)
  thresholdDown: number;
  currentPrice:  BN;       // current Pyth price (raw, exponent=-8)
  basePrice:     BN;       // last confirmed base price
  twap:          BN;
  priceChangePct: number;  // approximate % change from base
}

export interface InfoPoolClientOptions {
  infoProgramId?: PublicKey;
  poolProgramId?: PublicKey;
}

export class InfoPoolClient {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private program: any;
  private poolOwner:     PublicKey;
  private infoProgramId: PublicKey;
  private poolProgramId: PublicKey;

  constructor(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    program:   any,
    poolOwner: PublicKey,
    opts:      InfoPoolClientOptions = {}
  ) {
    this.program       = program;
    this.poolOwner     = poolOwner;
    this.infoProgramId = opts.infoProgramId ?? INFO_POOL_PROGRAM_ID;
    this.poolProgramId = opts.poolProgramId ?? POOL_PROGRAM_ID;
  }

  // ── PDA HELPERS ───────────────────────────────────────────────

  getInfoPoolPDA(): [PublicKey, number] {
    const [pool] = findPoolPDA(this.poolOwner, this.poolProgramId);
    return findInfoPoolPDA(pool, this.infoProgramId);
  }

  getAssetPDA(mint: PublicKey): [PublicKey, number] {
    const [pool] = findPoolPDA(this.poolOwner, this.poolProgramId);
    return findAssetPDA(pool, mint, this.poolProgramId);
  }

  // ── READ — fetch full InfoPool on-chain state ─────────────────
  async fetchInfoPool() {
    const [infoPda] = this.getInfoPoolPDA();
    return this.program.account.infoPoolAccount.fetch(infoPda);
  }

  // ── READ — current fee for a specific asset ───────────────────
  // Calls the on-chain read instruction via CPI simulation.
  // Returns fee in basis points (e.g. 25 = 0.25%).
  async getAssetFee(mint: PublicKey): Promise<number> {
    const [infoPool] = this.getInfoPoolPDA();

    const result = await this.program.methods
      .getAssetFee(mint)
      .accounts({ infoPool })
      .view();

    return result as number;
  }

  // ── READ — threshold state for a specific asset ───────────────
  async getThresholdState(mint: PublicKey) {
    const [infoPool] = this.getInfoPoolPDA();

    const result = await this.program.methods
      .getThresholdState(mint)
      .accounts({ infoPool })
      .view();

    return result;
  }

  // ── READ — full pool state ────────────────────────────────────
  async getPoolState() {
    const [infoPool] = this.getInfoPoolPDA();

    const result = await this.program.methods
      .getPoolState()
      .accounts({ infoPool })
      .view();

    return result;
  }

  // ── CRANK — update Pyth feeds (per-block) ────────────────────
  // Called by the off-chain crank every ~400ms.
  // Writes new TWAP/EMA values from the Pyth PriceUpdateV2 account.
  async updatePythFeeds(
    mint:          PublicKey,
    priceUpdatePda: PublicKey,
    cranker:       PublicKey
  ) {
    const [infoPool] = this.getInfoPoolPDA();

    const builder = this.program.methods
      .updatePythFeeds(mint)
      .accounts({
        infoPool,
        priceUpdate: priceUpdatePda,
        crank:       cranker,
      });

    return { builder };
  }

  // ── CRANK — run threshold check (per-block) ───────────────────
  // Evaluates 3-layer engine, fires CPI to Pool if block/unblock needed.
  async runThresholdCheck(
    mint:        PublicKey,
    poolPda:     PublicKey,
    assetPda:    PublicKey,
    cranker:     PublicKey
  ) {
    const [infoPool] = this.getInfoPoolPDA();

    const builder = this.program.methods
      .runThresholdCheck(mint)
      .accounts({
        infoPool,
        poolProgram:  this.poolProgramId,
        poolAccount:  poolPda,
        assetAccount: assetPda,
        crank:        cranker,
      });

    return { builder };
  }

  // ── CRANK — push 24h volume (per-minute) ─────────────────────
  // Updates volume_24h from DexScreener. Rotates prev ← current.
  async pushVolume(
    mint:      PublicKey,
    volume24h: BN,
    cranker:   PublicKey
  ) {
    const [infoPool] = this.getInfoPoolPDA();

    const builder = this.program.methods
      .pushVolume(mint, volume24h)
      .accounts({
        infoPool,
        crank: cranker,
      });

    return { builder };
  }

  // ── GOVERNANCE — set Pyth feed ID ────────────────────────────
  // Must be called after governance_add_asset for each asset.
  async setPythFeedId(
    mint:        PublicKey,
    pythFeedId:  number[],  // 32-byte array
    authority:   PublicKey
  ) {
    const [infoPool] = this.getInfoPoolPDA();

    const builder = this.program.methods
      .governanceSetPythFeedId(mint, pythFeedId)
      .accounts({
        infoPool,
        authority,
      });

    return { builder };
  }
}
