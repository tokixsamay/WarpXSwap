import {
  Connection,
  PublicKey,
  Transaction,
  TransactionInstruction,
  SystemProgram,
  Keypair,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddress,
  createAssociatedTokenAccountInstruction,
} from "@solana/spl-token";
import { AnchorProvider, Program, BN, Wallet } from "@coral-xyz/anchor";
import { POOL_PROGRAM_ID, FEE_SCALE } from "./constants";
import { findPoolPda, findAssetPda, findLpDepositPda } from "./pdas";
import {
  PoolAccount,
  AssetAccount,
  LpDepositAccount,
  AddAssetParams,
  ClaimableResult,
} from "./types";

export class PoolClient {
  private program: Program;
  private provider: AnchorProvider;

  constructor(program: Program, provider: AnchorProvider) {
    this.program = program;
    this.provider = provider;
  }

  // ── Pool state fetchers ───────────────────────────────────

  async fetchPool(authority: PublicKey): Promise<PoolAccount> {
    const [poolPda] = findPoolPda(authority);
    return this.program.account["poolAccount"].fetch(poolPda) as Promise<PoolAccount>;
  }

  async fetchAsset(pool: PublicKey, mint: PublicKey): Promise<AssetAccount> {
    const [assetPda] = findAssetPda(pool, mint);
    return this.program.account["assetAccount"].fetch(assetPda) as Promise<AssetAccount>;
  }

  async fetchLpDeposit(
    pool: PublicKey,
    mint: PublicKey,
    depositor: PublicKey,
  ): Promise<LpDepositAccount> {
    const [lpPda] = findLpDepositPda(pool, mint, depositor);
    return this.program.account["lpDepositAccount"].fetch(lpPda) as Promise<LpDepositAccount>;
  }

  // ── Claimable fees (off-chain calculation) ───────────────
  // Mirrors the on-chain formula exactly:
  //   claimable = pending_fees + amount × (pool_fps − fee_debt) / FEE_SCALE

  computeClaimable(
    poolFps: bigint,
    lpAmount: bigint,
    feeDebt: bigint,
    pendingFees: bigint,
  ): ClaimableResult {
    const fpsDelta    = poolFps > feeDebt ? poolFps - feeDebt : 0n;
    const fullAccrued = (lpAmount * fpsDelta) / FEE_SCALE;
    const claimable   = pendingFees + fullAccrued;
    return { claimable, principal: lpAmount, poolFps, feeDebt };
  }

  async getClaimable(
    pool: PublicKey,
    mint: PublicKey,
    depositor: PublicKey,
  ): Promise<ClaimableResult> {
    const [poolAcc, lpAcc] = await Promise.all([
      this.program.account["poolAccount"].fetch(pool) as Promise<PoolAccount>,
      this.fetchLpDeposit(pool, mint, depositor),
    ]);
    return this.computeClaimable(
      BigInt(poolAcc.poolFps.toString()),
      BigInt(lpAcc.amount.toString()),
      BigInt(lpAcc.feeDebt.toString()),
      BigInt(lpAcc.pendingFees.toString()),
    );
  }

  // ── Instructions ─────────────────────────────────────────

  async initializePool(
    authority: Keypair,
    baseAssetMint: PublicKey,
    poolType: "public" | "private",
  ): Promise<string> {
    const [poolPda] = findPoolPda(authority.publicKey);
    const poolTypeArg = poolType === "public"
      ? { public: {} }
      : { private: {} };

    return this.program.methods
      .initializePool(poolTypeArg)
      .accounts({
        pool:           poolPda,
        baseAssetMint,
        authority:      authority.publicKey,
        systemProgram:  SystemProgram.programId,
      })
      .signers([authority])
      .rpc();
  }

  async addAsset(
    authority: Keypair,
    params: AddAssetParams,
  ): Promise<string> {
    const [poolPda] = findPoolPda(authority.publicKey);
    const [assetPda] = findAssetPda(poolPda, params.mint);

    return this.program.methods
      .addAsset({
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
      })
      .accounts({
        pool:          poolPda,
        asset:         assetPda,
        authority:     authority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([authority])
      .rpc();
  }

  async deposit(
    user: Keypair,
    poolOwner: PublicKey,
    mint: PublicKey,
    amount: BN,
    userTokenAccount?: PublicKey,
  ): Promise<string> {
    const [poolPda]  = findPoolPda(poolOwner);
    const [assetPda] = findAssetPda(poolPda, mint);
    const [lpPda]    = findLpDepositPda(poolPda, mint, user.publicKey);

    const userAta = userTokenAccount
      ?? await getAssociatedTokenAddress(mint, user.publicKey);

    const poolVault = await getAssociatedTokenAddress(mint, poolPda, true);

    return this.program.methods
      .deposit(amount)
      .accounts({
        pool:          poolPda,
        asset:         assetPda,
        poolVault,
        userToken:     userAta,
        lpDeposit:     lpPda,
        user:          user.publicKey,
        tokenProgram:  TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([user])
      .rpc();
  }

  async publicExit(
    user: Keypair,
    poolOwner: PublicKey,
    mint: PublicKey,
    amount: BN,
    userTokenAccount?: PublicKey,
  ): Promise<string> {
    const [poolPda]  = findPoolPda(poolOwner);
    const [assetPda] = findAssetPda(poolPda, mint);
    const [lpPda]    = findLpDepositPda(poolPda, mint, user.publicKey);

    const userAta   = userTokenAccount
      ?? await getAssociatedTokenAddress(mint, user.publicKey);
    const poolVault = await getAssociatedTokenAddress(mint, poolPda, true);

    return this.program.methods
      .publicExit(amount)
      .accounts({
        pool:         poolPda,
        asset:        assetPda,
        poolVault,
        userToken:    userAta,
        lpDeposit:    lpPda,
        user:         user.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([user])
      .rpc();
  }

  async claimFees(
    user: Keypair,
    poolOwner: PublicKey,
    mint: PublicKey,
    userTokenAccount?: PublicKey,
  ): Promise<string> {
    const [poolPda]  = findPoolPda(poolOwner);
    const [assetPda] = findAssetPda(poolPda, mint);
    const [lpPda]    = findLpDepositPda(poolPda, mint, user.publicKey);

    const userAta   = userTokenAccount
      ?? await getAssociatedTokenAddress(mint, user.publicKey);
    const poolVault = await getAssociatedTokenAddress(mint, poolPda, true);

    return this.program.methods
      .claimFees()
      .accounts({
        pool:         poolPda,
        asset:        assetPda,
        poolVault,
        userToken:    userAta,
        lpDeposit:    lpPda,
        user:         user.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([user])
      .rpc();
  }

  async compoundFees(
    user: Keypair,
    poolOwner: PublicKey,
    mint: PublicKey,
  ): Promise<string> {
    const [poolPda]  = findPoolPda(poolOwner);
    const [assetPda] = findAssetPda(poolPda, mint);
    const [lpPda]    = findLpDepositPda(poolPda, mint, user.publicKey);

    return this.program.methods
      .compoundFees()
      .accounts({
        pool:      poolPda,
        asset:     assetPda,
        lpDeposit: lpPda,
        user:      user.publicKey,
      })
      .signers([user])
      .rpc();
  }

  async swap(
    user: Keypair,
    poolOwner: PublicKey,
    mintIn: PublicKey,
    mintOut: PublicKey,
    amountIn: BN,
    minAmountOut: BN,
    userTokenIn?: PublicKey,
    userTokenOut?: PublicKey,
  ): Promise<string> {
    const [poolPda]     = findPoolPda(poolOwner);
    const [assetInPda]  = findAssetPda(poolPda, mintIn);
    const [assetOutPda] = findAssetPda(poolPda, mintOut);

    const userAtaIn  = userTokenIn
      ?? await getAssociatedTokenAddress(mintIn, user.publicKey);
    const userAtaOut = userTokenOut
      ?? await getAssociatedTokenAddress(mintOut, user.publicKey);

    const poolVaultIn  = await getAssociatedTokenAddress(mintIn,  poolPda, true);
    const poolVaultOut = await getAssociatedTokenAddress(mintOut, poolPda, true);

    return this.program.methods
      .swap(amountIn, minAmountOut)
      .accounts({
        pool:         poolPda,
        assetOut:     assetOutPda,
        assetIn:      assetInPda,
        poolVaultOut,
        poolVaultIn,
        userTokenOut: userAtaOut,
        userTokenIn:  userAtaIn,
        user:         user.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([user])
      .rpc();
  }

  async setAllowance(
    authority: Keypair,
    poolOwner: PublicKey,
    assetMint: PublicKey,
    targetMint: PublicKey,
    allowed: boolean,
  ): Promise<string> {
    const [poolPda]  = findPoolPda(poolOwner);
    const [assetPda] = findAssetPda(poolPda, assetMint);

    return this.program.methods
      .setAllowance(targetMint, allowed)
      .accounts({
        pool:      poolPda,
        asset:     assetPda,
        authority: authority.publicKey,
      })
      .signers([authority])
      .rpc();
  }

  // ── Withdraw helpers (private pool only) ─────────────────

  async withdrawBase(
    authority: Keypair,
    poolOwner: PublicKey,
    baseMint: PublicKey,
    amount: BN,
    userTokenAccount?: PublicKey,
  ): Promise<string> {
    const [poolPda]  = findPoolPda(poolOwner);
    const [assetPda] = findAssetPda(poolPda, baseMint);

    const userAta   = userTokenAccount
      ?? await getAssociatedTokenAddress(baseMint, authority.publicKey);
    const poolVault = await getAssociatedTokenAddress(baseMint, poolPda, true);

    return this.program.methods
      .withdrawBase(amount)
      .accounts({
        pool:          poolPda,
        baseAsset:     assetPda,
        poolVaultBase: poolVault,
        userTokenBase: userAta,
        authority:     authority.publicKey,
        tokenProgram:  TOKEN_PROGRAM_ID,
      })
      .signers([authority])
      .rpc();
  }
      }
    
