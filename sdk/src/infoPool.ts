import { PublicKey } from "@solana/web3.js";
import { Program } from "@coral-xyz/anchor";
import { findInfoPoolPda } from "./pdas";
import { INFO_POOL_PROGRAM_ID, FEE_SCALE, BPS_DENOMINATOR, FEE_SENSITIVITY } from "./constants";
import { InfoPoolAccount, AssetInfo, ThresholdState } from "./types";

export class InfoPoolClient {
  private program: Program;

  constructor(program: Program) {
    this.program = program;
  }

  // ── State fetchers ────────────────────────────────────────

  async fetchInfoPool(pool: PublicKey): Promise<InfoPoolAccount> {
    const [pda] = findInfoPoolPda(pool);
    return this.program.account["infoPoolAccount"].fetch(pda) as Promise<InfoPoolAccount>;
  }

  async fetchAssetInfo(pool: PublicKey, mint: PublicKey): Promise<AssetInfo | undefined> {
    const info = await this.fetchInfoPool(pool);
    return info.assets.find(a => a.mint.equals(mint));
  }

  // ── Fee calculation (mirrors on-chain utils.rs) ───────────
  // V-shape fee curve for volatile assets:
  //   strength      = min(|growth_bps|, threshold_bps) × FEE_SENSITIVITY
  //                   / (threshold_bps × 100)
  //   fee_reduction = (fee_max − fee_min) × strength / 100
  //   fee           = clamp(fee_max − fee_reduction, fee_min, fee_max)

  computeVolatileFee(
    currentPrice: bigint,
    currentBase: bigint,
    thresholdUpBps: number,
    thresholdDownBps: number,
    feeMin: number,
    feeMax: number,
  ): number {
    if (currentBase === 0n) return feeMax;

    const priceDiff = currentPrice - currentBase;
    const growthBps = (priceDiff * BigInt(10_000)) / currentBase;

    const isUp        = growthBps >= 0n;
    const absGrowth   = growthBps < 0n ? -growthBps : growthBps;
    const threshBps   = BigInt(isUp ? thresholdUpBps : thresholdDownBps);

    if (threshBps === 0n) return feeMax;

    const capped   = absGrowth < threshBps ? absGrowth : threshBps;
    const strength = (capped * BigInt(FEE_SENSITIVITY)) / (threshBps * 100n);
    const reduction = (BigInt(feeMax - feeMin) * strength) / 100n;
    const fee = BigInt(feeMax) - reduction;

    const clipped = fee < BigInt(feeMin) ? BigInt(feeMin)
                  : fee > BigInt(feeMax) ? BigInt(feeMax)
                  : fee;
    return Number(clipped);
  }

  // ── Threshold state helpers ───────────────────────────────

  thresholdToLabel(state: ThresholdState): string {
    if ("neutral"        in state) return "Neutral";
    if ("exceededUp"     in state) return "ExceededUp";
    if ("exceededDown"   in state) return "ExceededDown";
    if ("approachingUp"  in state) return `ApproachingUp(${state.approachingUp}%)`;
    if ("approachingDown" in state) return `ApproachingDown(${state.approachingDown}%)`;
    return "Unknown";
  }

  isBlocking(state: ThresholdState): boolean {
    return "exceededUp" in state || "exceededDown" in state;
  }

  thresholdPct(state: ThresholdState): number {
    if ("neutral"        in state) return 0;
    if ("exceededUp"     in state) return 100;
    if ("exceededDown"   in state) return -100;
    if ("approachingUp"  in state) return state.approachingUp;
    if ("approachingDown" in state) return -state.approachingDown;
    return 0;
  }

  // ── Pool summary ──────────────────────────────────────────

  async getPoolSummary(pool: PublicKey): Promise<{
    poolId:      PublicKey;
    poolSize:    bigint;
    poolWeight:  bigint;
    lastUpdated: number;
    assets:      Array<{
      mint:           PublicKey;
      currentFee:     number;
      isBlocked:      boolean;
      thresholdState: string;
      allConfirmed:   boolean;
      price:          bigint;
    }>;
  }> {
    const info = await this.fetchInfoPool(pool);
    return {
      poolId:      info.poolId,
      poolSize:    BigInt(info.poolSize.toString()),
      poolWeight:  BigInt(info.poolWeight.toString()),
      lastUpdated: Number(info.lastUpdated.toString()),
      assets:      info.assets.map(a => ({
        mint:           a.mint,
        currentFee:     a.currentFee,
        isBlocked:      a.isBlocked,
        thresholdState: this.thresholdToLabel(a.thresholdState),
        allConfirmed:   a.layerStatus.allConfirmed,
        price:          BigInt(a.pythData.price.toString()),
      })),
    };
  }
                       }
  
