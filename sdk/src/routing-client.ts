import { PublicKey } from "@solana/web3.js";
import { BN } from "@coral-xyz/anchor";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { findPoolPDA, findAssetPDA, findInfoPoolPDA } from "./pda";
import { ROUTING_PROGRAM_ID, POOL_PROGRAM_ID, INFO_POOL_PROGRAM_ID } from "./constants";

// ═══════════════════════════════════════════════════════════════════
// ROUTING CLIENT
//
// Wraps Routing program: find best pool, get quotes, execute routes.
// Oracle rates are sourced on-chain from InfoPool — callers do NOT
// supply rateIn / rateOut.
//
// Usage:
//   const { routing } = createPrograms(provider);
//   const client = new RoutingClient(routing);
//
//   // Get a quote: how much SOL for 100 USDC?
//   const quote = await client.getQuote(poolOwner, {
//     assetIn:  usdcMint,
//     assetOut: solMint,
//     amountIn: new BN(100_000_000),  // 100 USDC
//   });
//
//   // Execute via best pool
//   const { builder } = await client.executeRoute({ ...params });
//   await builder.rpc();
// ═══════════════════════════════════════════════════════════════════

export interface QuoteParams {
  assetIn:     PublicKey;
  assetOut:    PublicKey;
  amountIn:    BN;
}

export interface QuoteResult {
  amountOut:   BN;
  feeBps:      number;
  feeAmount:   BN;
  priceImpact: number; // always ~0 (oracle-priced, no slippage curve)
  poolOwner:   PublicKey | null;
}

export interface ExecuteRouteParams {
  poolOwner:     PublicKey;
  assetInMint:   PublicKey;
  assetOutMint:  PublicKey;
  amountIn:      BN;
  minAmountOut:  BN;
  userTokenIn:   PublicKey;
  userTokenOut:  PublicKey;
  poolVaultIn:   PublicKey;
  poolVaultOut:  PublicKey;
  user:          PublicKey;
}

export interface RoutingClientOptions {
  routingProgramId?: PublicKey;
  poolProgramId?:    PublicKey;
  infoProgramId?:    PublicKey;
}

export class RoutingClient {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private program: any;
  private routingProgramId: PublicKey;
  private poolProgramId:    PublicKey;
  private infoProgramId:    PublicKey;

  constructor(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    program: any,
    opts:    RoutingClientOptions = {}
  ) {
    this.program           = program;
    this.routingProgramId  = opts.routingProgramId ?? ROUTING_PROGRAM_ID;
    this.poolProgramId     = opts.poolProgramId    ?? POOL_PROGRAM_ID;
    this.infoProgramId     = opts.infoProgramId    ?? INFO_POOL_PROGRAM_ID;
  }

  // ── GET QUOTE (on-chain CPI simulation) ──────────────────────
  // Oracle rates are sourced from InfoPool on-chain — not caller-supplied.
  async getQuote(
    poolOwner: PublicKey,
    params:    QuoteParams
  ): Promise<QuoteResult> {
    const [pool]     = findPoolPDA(poolOwner, this.poolProgramId);
    const [infoPool] = findInfoPoolPDA(pool, this.infoProgramId);
    const [assetIn]  = findAssetPDA(pool, params.assetIn,  this.poolProgramId);
    const [assetOut] = findAssetPDA(pool, params.assetOut, this.poolProgramId);

    const result = await this.program.methods
      .getQuote({
        assetIn:  params.assetIn,
        assetOut: params.assetOut,
        amountIn: params.amountIn,
      })
      .accounts({
        pool,
        infoPool,
        assetIn,
        assetOut,
        poolProgram:     this.poolProgramId,
        infoPoolProgram: this.infoProgramId,
      })
      .view();

    return {
      amountOut:   new BN(result.amountOut.toString()),
      feeBps:      result.feeBps,
      feeAmount:   new BN(result.feeAmount.toString()),
      priceImpact: 0,
      poolOwner,
    };
  }

  // ── EXECUTE ROUTE (single-hop) ────────────────────────────────
  // Finds best pool and executes swap in one transaction.
  async executeRoute(params: ExecuteRouteParams) {
    const [pool]     = findPoolPDA(params.poolOwner, this.poolProgramId);
    const [infoPool] = findInfoPoolPDA(pool, this.infoProgramId);
    const [assetIn]  = findAssetPDA(pool, params.assetInMint,  this.poolProgramId);
    const [assetOut] = findAssetPDA(pool, params.assetOutMint, this.poolProgramId);

    const builder = this.program.methods
      .executeRoute({
        assetIn:      params.assetInMint,
        assetOut:     params.assetOutMint,
        amountIn:     params.amountIn,
        minAmountOut: params.minAmountOut,
      })
      .accounts({
        pool,
        infoPool,
        assetIn,
        assetOut,
        poolVaultIn:     params.poolVaultIn,
        poolVaultOut:    params.poolVaultOut,
        userTokenIn:     params.userTokenIn,
        userTokenOut:    params.userTokenOut,
        user:            params.user,
        poolProgram:     this.poolProgramId,
        infoPoolProgram: this.infoProgramId,
        tokenProgram:    TOKEN_PROGRAM_ID,
      });

    return { builder };
  }

}
