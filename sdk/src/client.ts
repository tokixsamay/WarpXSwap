// eslint-disable-next-line @typescript-eslint/no-unused-vars
import type { Program } from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  GOVERNANCE_PROGRAM_ID,
  POOL_PROGRAM_ID,
  INFO_POOL_PROGRAM_ID,
} from "./constants";
import {
  findGovernancePDA,
  findProposalPDA,
  findPoolPDA,
  findAssetPDA,
  findInfoPoolPDA,
} from "./pda";
import type {
  AddAssetPayload,
  RemoveAssetPayload,
  UpdateAllowancePayload,
  UpdateMaxPctPayload,
  UpdateThresholdPayload,
  UpdateFeeRangePayload,
  SetPythFeedIdPayload,
  SetInflowBlockedPayload,
  ExecuteProposalAccounts,
  ExecuteResult,
} from "./types";

// ═══════════════════════════════════════════════════════════════
// GOVERNANCE CLIENT
//
// Usage:
//   import governanceIdl from "../target/idl/governance_program.json";
//   const program = new Program(governanceIdl, provider);
//   const client  = new GovernanceClient(program, poolOwner);
//
//   // Execute a passed UpdateFeeRange proposal:
//   const { builder } = await client.executeUpdateFeeRange(proposalId, {
//     kind: "UpdateFeeRange",
//     mint: new PublicKey("..."),
//     newMin: 50,
//     newMax: 200,
//   });
//   await builder.rpc();
//
// The poolId is the pool's owner pubkey — it's the seed for the Pool PDA.
// Pass `poolProgramId` / `infoProgramId` only to override defaults (e.g. devnet
// deployments with different addresses).
// ═══════════════════════════════════════════════════════════════

export interface GovernanceClientOptions {
  poolProgramId?: PublicKey;
  infoProgramId?: PublicKey;
  governanceProgramId?: PublicKey;
}

export class GovernanceClient {
  // typed as `any` to avoid TS2589 (infinite generic depth in Anchor's Program<T>)
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  private program: any;
  /** Owner of the Pool PDA — used as the `pool` seed */
  private poolOwner: PublicKey;
  private poolProgramId: PublicKey;
  private infoProgramId: PublicKey;
  private governanceProgramId: PublicKey;

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  constructor(
    program: any, // Program<GovernanceIDL> — pass the result of `new Program(idl, provider)`
    poolOwner: PublicKey,
    opts: GovernanceClientOptions = {}
  ) {
    this.program      = program;
    this.poolOwner    = poolOwner;
    this.poolProgramId        = opts.poolProgramId        ?? POOL_PROGRAM_ID;
    this.infoProgramId        = opts.infoProgramId        ?? INFO_POOL_PROGRAM_ID;
    this.governanceProgramId  = opts.governanceProgramId  ?? GOVERNANCE_PROGRAM_ID;
  }

  // ── SHARED ACCOUNT BUILDERS ──────────────────────────────────

  private baseAccounts(proposalId: BN): {
    governance: PublicKey;
    proposal:   PublicKey;
    poolId:     PublicKey;
    pool:       PublicKey;
    infoPool:   PublicKey;
  } {
    const [pool]      = findPoolPDA(this.poolOwner, this.poolProgramId);
    const poolId      = pool;
    const [governance] = findGovernancePDA(poolId, this.governanceProgramId);
    const [proposal]   = findProposalPDA(poolId, proposalId, this.governanceProgramId);
    const [infoPool]   = findInfoPoolPDA(poolId, this.infoProgramId);
    return { governance, proposal, poolId, pool, infoPool };
  }

  private buildAccounts(
    base: ReturnType<typeof this.baseAccounts>,
    assetMint: PublicKey,
    executor: PublicKey
  ): ExecuteProposalAccounts {
    const [asset] = findAssetPDA(base.pool, assetMint, this.poolProgramId);
    return {
      governance:      base.governance,
      proposal:        base.proposal,
      poolProgram:     this.poolProgramId,
      infoPoolProgram: this.infoProgramId,
      poolAccount:     base.pool,
      assetAccount:    asset,
      infoPoolAccount: base.infoPool,
      executor,
      systemProgram:   SystemProgram.programId,
    };
  }

  private makeBuilder(
    proposalId: BN,
    accounts: ExecuteProposalAccounts
  ): ExecuteResult["builder"] {
    return this.program.methods
      .executeProposal(proposalId)
      .accounts({
        governance:      accounts.governance,
        proposal:        accounts.proposal,
        poolProgram:     accounts.poolProgram,
        infoPoolProgram: accounts.infoPoolProgram,
        poolAccount:     accounts.poolAccount,
        assetAccount:    accounts.assetAccount,
        infoPoolAccount: accounts.infoPoolAccount,
        executor:        accounts.executor,
        systemProgram:   accounts.systemProgram,
      });
  }

  // ── executeUpdateFeeRange ─────────────────────────────────────
  // Executes a passed UpdateFeeRange proposal.
  // Dual-writes to Pool AssetAccount + Info Pool AssetInfo.
  async executeUpdateFeeRange(
    proposalId: BN,
    payload: UpdateFeeRangePayload,
    executor: PublicKey
  ): Promise<ExecuteResult> {
    const base     = this.baseAccounts(proposalId);
    const accounts = this.buildAccounts(base, payload.mint, executor);
    const builder  = this.makeBuilder(proposalId, accounts);
    return { builder, accounts };
  }

  // ── executeUpdateThreshold ────────────────────────────────────
  // Executes a passed UpdateThreshold proposal.
  // Dual-writes to Pool AssetAccount + Info Pool AssetInfo.
  // Resets threshold state to Neutral and clears layer confirmations.
  async executeUpdateThreshold(
    proposalId: BN,
    payload: UpdateThresholdPayload,
    executor: PublicKey
  ): Promise<ExecuteResult> {
    const base     = this.baseAccounts(proposalId);
    const accounts = this.buildAccounts(base, payload.mint, executor);
    const builder  = this.makeBuilder(proposalId, accounts);
    return { builder, accounts };
  }

  // ── executeUpdateMaxPct ───────────────────────────────────────
  // Executes a passed UpdateMaxPct proposal.
  // Dual-writes to Pool AssetAccount + Info Pool AssetInfo.
  async executeUpdateMaxPct(
    proposalId: BN,
    payload: UpdateMaxPctPayload,
    executor: PublicKey
  ): Promise<ExecuteResult> {
    const base     = this.baseAccounts(proposalId);
    const accounts = this.buildAccounts(base, payload.mint, executor);
    const builder  = this.makeBuilder(proposalId, accounts);
    return { builder, accounts };
  }

  // ── executeAddAsset ───────────────────────────────────────────
  // Executes a passed AddAsset proposal.
  // Inits a new AssetAccount PDA (executor pays rent).
  // Registers the asset in Info Pool's 3-layer Pyth engine.
  // NOTE: The asset PDA is derived but does not yet exist on-chain —
  // the Pool program's `init` constraint creates it during execution.
  async executeAddAsset(
    proposalId: BN,
    payload: AddAssetPayload,
    executor: PublicKey
  ): Promise<ExecuteResult> {
    const base     = this.baseAccounts(proposalId);
    const accounts = this.buildAccounts(base, payload.mint, executor);
    const builder  = this.makeBuilder(proposalId, accounts);
    return { builder, accounts };
  }

  // ── executeRemoveAsset ────────────────────────────────────────
  // Executes a passed RemoveAsset proposal.
  // Closes the AssetAccount PDA — rent is returned to executor.
  // Removes the asset from Info Pool's 3-layer Pyth engine.
  // Pool enforces: asset.amount == 0 and mint != base_asset.
  async executeRemoveAsset(
    proposalId: BN,
    payload: RemoveAssetPayload,
    executor: PublicKey
  ): Promise<ExecuteResult> {
    const base     = this.baseAccounts(proposalId);
    const accounts = this.buildAccounts(base, payload.mint, executor);
    const builder  = this.makeBuilder(proposalId, accounts);
    return { builder, accounts };
  }

  // ── executeUpdateAllowance ────────────────────────────────────
  // Executes a passed UpdateAllowance proposal.
  // `payload.asset` = source mint whose allowed list is changed.
  // `payload.target` = mint being added or removed.
  // Dual-writes to Pool AssetAccount + Info Pool AssetInfo.
  async executeUpdateAllowance(
    proposalId: BN,
    payload: UpdateAllowancePayload,
    executor: PublicKey
  ): Promise<ExecuteResult> {
    const base     = this.baseAccounts(proposalId);
    const accounts = this.buildAccounts(base, payload.asset, executor);
    const builder  = this.makeBuilder(proposalId, accounts);
    return { builder, accounts };
  }

  // ── executeSetPythFeedId ──────────────────────────────────────
  // Executes a passed SetPythFeedId proposal.
  // Rotates the per-asset Pyth V2 feed ID on InfoPool.
  async executeSetPythFeedId(
    proposalId: BN,
    payload: SetPythFeedIdPayload,
    executor: PublicKey
  ): Promise<ExecuteResult> {
    const base     = this.baseAccounts(proposalId);
    const accounts = this.buildAccounts(base, payload.mint, executor);
    const builder  = this.makeBuilder(proposalId, accounts);
    return { builder, accounts };
  }

  // ── executeSetInflowBlocked ───────────────────────────────────
  // Executes a passed SetInflowBlocked proposal.
  // Manually blocks or unblocks inflow for an asset (emergency circuit-breaker).
  async executeSetInflowBlocked(
    proposalId: BN,
    payload: SetInflowBlockedPayload,
    executor: PublicKey
  ): Promise<ExecuteResult> {
    const base     = this.baseAccounts(proposalId);
    const accounts = this.buildAccounts(base, payload.mint, executor);
    const builder  = this.makeBuilder(proposalId, accounts);
    return { builder, accounts };
  }

  // ── executeProposal (generic) ─────────────────────────────────
  // Dispatches to the correct typed method based on payload.kind.
  // Useful when iterating over a queue of passed proposals.
  async executeProposal(
    proposalId: BN,
    payload: import("./types").ProposalPayload,
    executor: PublicKey
  ): Promise<ExecuteResult> {
    switch (payload.kind) {
      case "UpdateFeeRange":
        return this.executeUpdateFeeRange(proposalId, payload, executor);
      case "UpdateThreshold":
        return this.executeUpdateThreshold(proposalId, payload, executor);
      case "UpdateMaxPct":
        return this.executeUpdateMaxPct(proposalId, payload, executor);
      case "AddAsset":
        return this.executeAddAsset(proposalId, payload, executor);
      case "RemoveAsset":
        return this.executeRemoveAsset(proposalId, payload, executor);
      case "UpdateAllowance":
        return this.executeUpdateAllowance(proposalId, payload, executor);
      case "SetPythFeedId":
        return this.executeSetPythFeedId(proposalId, payload, executor);
      case "SetInflowBlocked":
        return this.executeSetInflowBlocked(proposalId, payload, executor);
    }
  }

  // ── PDA HELPERS (exposed for caller convenience) ─────────────

  getGovernancePDA(poolId: PublicKey): [PublicKey, number] {
    return findGovernancePDA(poolId, this.governanceProgramId);
  }

  getProposalPDA(poolId: PublicKey, proposalId: BN): [PublicKey, number] {
    return findProposalPDA(poolId, proposalId, this.governanceProgramId);
  }

  getPoolPDA(): [PublicKey, number] {
    return findPoolPDA(this.poolOwner, this.poolProgramId);
  }

  getAssetPDA(mint: PublicKey): [PublicKey, number] {
    const [pool] = this.getPoolPDA();
    return findAssetPDA(pool, mint, this.poolProgramId);
  }

  getInfoPoolPDA(poolId: PublicKey): [PublicKey, number] {
    return findInfoPoolPDA(poolId, this.infoProgramId);
  }
}
