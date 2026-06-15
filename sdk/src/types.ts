import { PublicKey } from "@solana/web3.js";
import BN from "bn.js";

// ── PROPOSAL PAYLOAD TYPES ─────────────────────────────────────
// Mirror of the on-chain ProposalPayload enum.
// Pass one of these to the matching GovernanceClient method.

export interface AddAssetPayload {
  kind: "AddAsset";
  mint: PublicKey;
  maxPctMin: number;
  maxPctMax: number;
  feeMin: number;
  feeMax: number;
  thresholdUp: number;
  thresholdDown: number;
  initialBase: BN;
  allowed: PublicKey[];
  isStable: boolean;
  staticFeeBps: number;
}

export interface RemoveAssetPayload {
  kind: "RemoveAsset";
  mint: PublicKey;
}

export interface UpdateAllowancePayload {
  kind: "UpdateAllowance";
  /** Mint of the asset whose allowance list is being updated */
  asset: PublicKey;
  /** Mint being added to or removed from the allowance list */
  target: PublicKey;
  allowed: boolean;
}

export interface UpdateMaxPctPayload {
  kind: "UpdateMaxPct";
  mint: PublicKey;
  newMin: number;
  newMax: number;
}

export interface UpdateThresholdPayload {
  kind: "UpdateThreshold";
  mint: PublicKey;
  newUp: number;
  newDown: number;
}

export interface UpdateFeeRangePayload {
  kind: "UpdateFeeRange";
  mint: PublicKey;
  newMin: number;
  newMax: number;
}

export interface SetPythFeedIdPayload {
  kind: "SetPythFeedId";
  mint: PublicKey;
  pythFeedId: number[];
}

export interface SetInflowBlockedPayload {
  kind: "SetInflowBlocked";
  mint: PublicKey;
  blocked: boolean;
}

export type ProposalPayload =
  | AddAssetPayload
  | RemoveAssetPayload
  | UpdateAllowancePayload
  | UpdateMaxPctPayload
  | UpdateThresholdPayload
  | UpdateFeeRangePayload
  | SetPythFeedIdPayload
  | SetInflowBlockedPayload;

// ── EXECUTE RESULT ─────────────────────────────────────────────
// Returned by every GovernanceClient.execute* method.
// `builder` is the Anchor MethodsBuilder for the execute_proposal instruction.
// Call .rpc() to send immediately, .instruction() to compose into a
// larger transaction, or .transaction() to get a raw Transaction.

export interface ExecuteResult {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  builder: any; // Program<any>.methods.executeProposal(...).accounts(...)
  /** Pre-computed accounts passed to the instruction */
  accounts: ExecuteProposalAccounts;
}

export interface ExecuteProposalAccounts {
  governance: PublicKey;
  proposal: PublicKey;
  poolProgram: PublicKey;
  infoPoolProgram: PublicKey;
  poolAccount: PublicKey;
  assetAccount: PublicKey;
  infoPoolAccount: PublicKey;
  executor: PublicKey;
  systemProgram: PublicKey;
  }
