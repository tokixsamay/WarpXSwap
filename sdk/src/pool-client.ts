import {
  PublicKey,
  Keypair,
  SystemProgram,
} from "@solana/web3.js";
import {
  getOrCreateAssociatedTokenAccount,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { BN } from "@coral-xyz/anchor";
import { findPoolPDA, findAssetPDA, findInfoPoolPDA } from "./pda";
import { POOL_PROGRAM_ID, INFO_POOL_PROGRAM_ID } from "./constants";

// ═══════════════════════════════════════════════════════════════════
// POOL CLIENT
//
// Wraps Pool program instructions: swap, deposit, withdraw, allowance.
//
// Usage:
//   const { pool, infoPool } = createPrograms(provider);
//   const client = new PoolClient(pool, infoPool, poolOwner);
//
//   // Swap: USDC → SOL using Pyth prices
//   const { builder } = await client.swap({
//     assetInMint:    usdcMint,
//     assetOutMint:   solMint,
//     amountIn:       new BN(100_000_000),   // 100 USDC (6 decimals)
//     minAmountOut:   new BN(650_000_000),   // min 0.65 SOL (9 decimals)
//     rateIn:         new BN(100_000_000),   // $1.00 × 10^8
//     rateOut:        new BN(15_000_000_000),// $150 × 10^8
//     userTokenIn:    userUsdcAta,
//     userTokenOut:   userSolAta,
//     poolVaultIn:    poolUsdcVault,
//     poolVaultOut:   poolSolVault,
//   });
//   await builder.rpc();
// ═══════════════════════════════════════════════════════════════════

export interface SwapParams {
  assetInMint:   PublicKey;
  assetOutMint:  PublicKey;
  amountIn:      BN;
  minAmountOut:  BN;
  userTokenIn:   PublicKey;  // user's ATA for asset_in
  userTokenOut:  PublicKey;  // user's ATA for asset_out
  poolVaultIn:   PublicKey;  // pool's token vault for asset_in
  poolVaultOut:  PublicKey;  // pool's token vault for asset_out
  user:          PublicKey;
}

export interface SwapResult {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  builder: any;
  estimatedOut: BN;
  feeEstimateBps: number;
}

export interface DepositParams {
  mint:             PublicKey;
  amount:           BN;
  userTokenAccount: PublicKey;
  poolVault:        PublicKey;
  user:             PublicKey;
}

export interface WithdrawBaseParams {
  mint:           PublicKey;
  amount:         BN;
  userTokenAccount: PublicKey;
  poolVault:      PublicKey;
  authority:      PublicKey;
}

export interface WithdrawAllParams {
  percentage:     number; // 1–100
  authority:      PublicKey;
  // Map of mint → { userAta, poolVault }
  assets: {
    mint:       PublicKey;
    userAta:    PublicKey;
    poolVault:  PublicKey;
  }[];
}

export interface SetAllowanceParams {
  assetMint:    PublicKey;
  targetMint:   PublicKey;
  allowed:      boolean;
  authority:    PublicKey;
}

export interface AddAssetParams {
  mint:            PublicKey;
  maxPctMin:       number;
  maxPctMax:       number;
  feeMin:          number;
  feeMax:          number;
  thresholdUp:     number;
  thresholdDown:   number;
  initialBase:     BN;
  allowed:         PublicKey[];
  isStable:        boolean;
  staticFeeBps:    number;
  authority:       PublicKey;
}

export interface AssetInfo {
  /** Mint address */
  mint:            PublicKey;
  /** On-chain PDA for this asset */
  assetPda:        PublicKey;
  /** Current token balance held in the pool vault */
  amount:          BN;
  /** Min concentration % allowed (e.g. 10 = 10%) */
  maxPctMin:       number;
  /** Max concentration % allowed (e.g. 35 = 35%) */
  maxPctMax:       number;
  /** Dynamic fee floor in bps */
  feeMin:          number;
  /** Dynamic fee ceiling in bps */
  feeMax:          number;
  /** Fee last computed by the 3-layer engine (bps) */
  currentFeeBps:   number;
  /** Upper IL threshold in bps */
  thresholdUp:     number;
  /** Lower IL threshold in bps */
  thresholdDown:   number;
  /** Reference oracle base price (Pyth-scale: price × 10^8) */
  currentBase:     BN;
  /** Latest oracle price pushed from InfoPool (Pyth-scale: price × 10^6) */
  oraclePrice:     BN;
  /** True when inflow is blocked (threshold breached + at max concentration) */
  inflowBlocked:   boolean;
  /** Raw threshold state from the program enum */
  thresholdState:  "normal" | "breachedUp" | "breachedDown";
  /** Mints this asset is allowed to swap with */
  allowed:         PublicKey[];
  /** True for stablecoins — bypasses V-shape fee and uses staticFeeBps */
  isStable:        boolean;
  /** Flat fee in bps used when isStable = true */
  staticFeeBps:    number;
  /**
   * The fee that will actually be charged on the next swap.
   * = staticFeeBps  when isStable
   * = currentFeeBps otherwise
   */
  effectiveFeeBps: number;
}

export interface PoolClientOptions {
  poolProgramId?:     PublicKey;
  infoProgramId?:     PublicKey;
}

export class PoolClient {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private poolProgram: any;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private infoProgram: any;
  private poolOwner:      PublicKey;
  private poolProgramId:  PublicKey;
  private infoProgramId:  PublicKey;

  constructor(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    poolProgram: any,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    infoProgram: any,
    poolOwner:   PublicKey,
    opts:        PoolClientOptions = {}
  ) {
    this.poolProgram  = poolProgram;
    this.infoProgram  = infoProgram;
    this.poolOwner    = poolOwner;
    this.poolProgramId = opts.poolProgramId ?? POOL_PROGRAM_ID;
    this.infoProgramId = opts.infoProgramId ?? INFO_POOL_PROGRAM_ID;
  }

  // ── PDA HELPERS ───────────────────────────────────────────────

  getPoolPDA(): [PublicKey, number] {
    return findPoolPDA(this.poolOwner, this.poolProgramId);
  }

  getAssetPDA(mint: PublicKey): [PublicKey, number] {
    const [pool] = this.getPoolPDA();
    return findAssetPDA(pool, mint, this.poolProgramId);
  }

  getInfoPoolPDA(): [PublicKey, number] {
    const [pool] = this.getPoolPDA();
    return findInfoPoolPDA(pool, this.infoProgramId);
  }

  // ── ESTIMATE SWAP OUTPUT ──────────────────────────────────────
  // Pure math — no chain call needed.
  // amount_out = (amount_in * rate_in) / rate_out - fee
  estimateSwapOut(
    amountIn:    BN,
    rateIn:      BN,
    rateOut:     BN,
    feeBps:      number = 25
  ): { gross: BN; fee: BN; net: BN } {
    const gross = amountIn.mul(rateIn).div(rateOut);
    const fee   = gross.muln(feeBps).divn(10_000);
    const net   = gross.sub(fee);
    return { gross, fee, net };
  }

  // ── SWAP ──────────────────────────────────────────────────────
  // Oracle-rate swap: trader brings asset_in, receives asset_out.
  // Allowance check: asset_out.allowed must contain asset_in.mint.
  async swap(params: SwapParams): Promise<SwapResult> {
    const [pool]     = this.getPoolPDA();
    const [assetOut] = this.getAssetPDA(params.assetOutMint);
    const [assetIn]  = this.getAssetPDA(params.assetInMint);

    const estimatedOut = this.estimateSwapOut(
      params.amountIn, new BN(1), new BN(1)
    );

    const builder = this.poolProgram.methods
      .swap(params.amountIn, params.minAmountOut)
      .accounts({
        pool,
        assetOut,
        assetIn,
        poolVaultOut:  params.poolVaultOut,
        poolVaultIn:   params.poolVaultIn,
        userTokenOut:  params.userTokenOut,
        userTokenIn:   params.userTokenIn,
        user:          params.user,
        tokenProgram:  TOKEN_PROGRAM_ID,
      });

    return {
      builder,
      estimatedOut: estimatedOut.net,
      feeEstimateBps: 25,
    };
  }

  // ── DEPOSIT ───────────────────────────────────────────────────
  // LP deposits base asset (or any asset) into the pool.
  async deposit(params: DepositParams) {
    const [pool]  = this.getPoolPDA();
    const [asset] = this.getAssetPDA(params.mint);

    const builder = this.poolProgram.methods
      .deposit(params.amount)
      .accounts({
        pool,
        asset,
        poolVault:    params.poolVault,
        userToken:    params.userTokenAccount,
        user:         params.user,
        tokenProgram: TOKEN_PROGRAM_ID,
      });

    return { builder };
  }

  // ── WITHDRAW BASE ─────────────────────────────────────────────
  // LP withdraws a specific amount of base asset.
  async withdrawBase(params: WithdrawBaseParams) {
    const [pool]  = this.getPoolPDA();
    const [asset] = this.getAssetPDA(params.mint);

    const builder = this.poolProgram.methods
      .withdrawBase(params.amount)
      .accounts({
        pool,
        baseAsset:      asset,
        poolVaultBase:  params.poolVault,
        userTokenBase:  params.userTokenAccount,
        authority:      params.authority,
        tokenProgram:   TOKEN_PROGRAM_ID,
      });

    return { builder };
  }

  // ── WITHDRAW ALL ──────────────────────────────────────────────
  // LP withdraws a percentage of all assets proportionally.
  // NOTE: For multi-asset withdrawal, send one tx per asset or use
  // the multi-asset CPI pattern. Simplified single-asset call here.
  async withdrawAll(params: WithdrawAllParams) {
    const [pool] = this.getPoolPDA();

    const builder = this.poolProgram.methods
      .withdrawAll(params.percentage)
      .accounts({
        pool,
        authority:    params.authority,
        tokenProgram: TOKEN_PROGRAM_ID,
      });

    return { builder };
  }

  // ── SET ALLOWANCE ─────────────────────────────────────────────
  // Pool owner: allow or disallow asset_in for a given asset_out.
  // Example: SOL.setAllowance(USDC, true) → traders can bring USDC to get SOL.
  async setAllowance(params: SetAllowanceParams) {
    const [pool]  = this.getPoolPDA();
    const [asset] = this.getAssetPDA(params.assetMint);

    const builder = this.poolProgram.methods
      .setAllowance(params.targetMint, params.allowed)
      .accounts({
        pool,
        asset,
        authority: params.authority,
      });

    return { builder };
  }

  // ── ADD ASSET ─────────────────────────────────────────────────
  // Pool owner: register a new asset in the pool.
  async addAsset(params: AddAssetParams) {
    const [pool] = this.getPoolPDA();

    const [assetPda] = this.getAssetPDA(params.mint);

    const anchorParams = {
      mint:          params.mint,
      maxPctMin:     params.maxPctMin,
      maxPctMax:     params.maxPctMax,
      feeMin:        params.feeMin,
      feeMax:        params.feeMax,
      thresholdUp:   params.thresholdUp,
      thresholdDown: params.thresholdDown,
      initialBase:   params.initialBase,
      allowed:       params.allowed,
      isStable:      params.isStable,
      staticFeeBps:  params.staticFeeBps,
    };

    const builder = this.poolProgram.methods
      .addAsset(anchorParams)
      .accounts({
        pool,
        asset:         assetPda,
        authority:     params.authority,
        systemProgram: SystemProgram.programId,
      });

    return { builder };
  }

  // ── REMOVE ASSET ──────────────────────────────────────────────
  // Pool owner: remove an asset (must have 0 balance).
  async removeAsset(mint: PublicKey, authority: PublicKey) {
    const [pool]  = this.getPoolPDA();
    const [asset] = this.getAssetPDA(mint);

    const builder = this.poolProgram.methods
      .removeAsset()
      .accounts({
        pool,
        asset,
        authority,
      });

    return { builder };
  }

  // ── CRANK: push oracle price from InfoPool into Pool ─────────
  // Calls info_pool.push_oracle_price_to_pool — the required middle
  // step between update_pyth_feeds and run_threshold_check.
  //
  // Without this, Pool's asset.oracle_price stays 0 and every swap
  // reverts with OraclePriceNotSet.
  //
  // `caller` is any Signer keypair (crank key, test wallet, etc.).
  //
  // Example:
  //   const { builder } = client.pushOraclePriceToPool(SOL_MINT, crankKey);
  //   await builder.rpc();
  pushOraclePriceToPool(mint: PublicKey, caller: PublicKey) {
    const [infoPool]     = this.getInfoPoolPDA();
    const [poolAccount]  = this.getPoolPDA();
    const [assetAccount] = this.getAssetPDA(mint);

    const builder = this.infoProgram.methods
      .pushOraclePriceToPool(mint)
      .accounts({
        infoPool,
        poolProgram:  this.poolProgramId,
        poolAccount,
        assetAccount,
        crank:        caller,
      });

    return { builder };
  }

  // ── READ: fetch raw on-chain asset account ───────────────────
  async fetchAsset(mint: PublicKey) {
    const [assetPda] = this.getAssetPDA(mint);
    return this.poolProgram.account.assetAccount.fetch(assetPda);
  }

  async fetchPool() {
    const [poolPda] = this.getPoolPDA();
    return this.poolProgram.account.poolAccount.fetch(poolPda);
  }

  // ── READ: typed live asset state ─────────────────────────────
  // Returns a clean snapshot of the fields operators and UIs care about:
  //   fee state, inflow-blocked flag, oracle base price, threshold state.
  //
  // Example:
  //   const info = await client.getAssetInfo(SOL_MINT);
  //   console.log(info.currentFeeBps, info.inflowBlocked, info.oraclePrice);
  async getAssetInfo(mint: PublicKey): Promise<AssetInfo> {
    const raw = await this.fetchAsset(mint);
    const [assetPda] = this.getAssetPDA(mint);

    // ThresholdState is an Anchor enum: { normal:{} } | { breachedUp:{} } | ...
    const thresholdStateKey = Object.keys(raw.thresholdState)[0] as AssetInfo["thresholdState"];

    return {
      mint:             raw.mint             as PublicKey,
      assetPda,
      amount:           raw.amount           as BN,
      maxPctMin:        raw.maxPctMin        as number,
      maxPctMax:        raw.maxPctMax        as number,
      feeMin:           raw.feeMin           as number,
      feeMax:           raw.feeMax           as number,
      currentFeeBps:    raw.currentFee       as number,
      thresholdUp:      raw.thresholdUp      as number,
      thresholdDown:    raw.thresholdDown    as number,
      currentBase:      raw.currentBase      as BN,
      oraclePrice:      raw.oraclePrice      as BN,
      inflowBlocked:    raw.isBlocked        as boolean,
      thresholdState:   thresholdStateKey,
      allowed:          raw.allowed          as PublicKey[],
      isStable:         raw.isStable         as boolean,
      staticFeeBps:     raw.staticFeeBps     as number,
      effectiveFeeBps:  raw.isStable ? raw.staticFeeBps as number : raw.currentFee as number,
    };
  }
}
