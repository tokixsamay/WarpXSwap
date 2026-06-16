import { PublicKey } from "@solana/web3.js";
import BN from "bn.js";

// ── Pool types ──────────────────────────────────────────────

export type PoolType = { public: Record<string, never> } | { private: Record<string, never> };

export interface PoolAccount {
  poolType:             PoolType;
  owner:                PublicKey;
  baseAsset:            PublicKey;
  totalValue:           BN;
  poolWeight:           BN;
  isActive:             boolean;
  bump:                 number;
  poolFps:              BN;
  poolTotalLpDeposited: BN;
}

export type ThresholdState =
  | { neutral: Record<string, never> }
  | { approachingUp: number }
  | { approachingDown: number }
  | { exceededUp: Record<string, never> }
  | { exceededDown: Record<string, never> };

export interface AssetAccount {
  pool:           PublicKey;
  mint:           PublicKey;
  amount:         BN;
  maxPctMin:      number;
  maxPctMax:      number;
  feeMin:         number;
  feeMax:         number;
  currentFee:     number;
  thresholdUp:    number;
  thresholdDown:  number;
  currentBase:    BN;
  allowed:        PublicKey[];
  isBlocked:      boolean;
  thresholdState: ThresholdState;
  oraclePrice:    BN;
  isStable:       boolean;
  staticFeeBps:   number;
  bump:           number;
  feesPerShare:   BN;
  totalDeposited: BN;
}

export interface LpDepositAccount {
  pool:        PublicKey;
  asset:       PublicKey;
  depositor:   PublicKey;
  amount:      BN;
  bump:        number;
  feeDebt:     BN;
  pendingFees: BN;
}

// ── AddAsset params ─────────────────────────────────────────

export interface AddAssetParams {
  mint:          PublicKey;
  maxPctMin:     number;
  maxPctMax:     number;
  feeMin:        number;
  feeMax:        number;
  thresholdUp:   number;
  thresholdDown: number;
  initialBase:   BN;
  allowed:       PublicKey[];
  isStable:      boolean;
  staticFeeBps:  number;
}

// ── InfoPool types ───────────────────────────────────────────

export interface LayerConfirmation {
  twapConfirmed:       boolean;
  volumeConfirmed:     boolean;
  confidenceConfirmed: boolean;
  allConfirmed:        boolean;
  lastConfirmed:       BN;
}

export interface PythFeedData {
  mint:        PublicKey;
  twapShort:   BN;
  twapMedium:  BN;
  twapLong:    BN;
  volume24h:   BN;
  volumePrev:  BN;
  confidence:  BN;
  price:       BN;
  lastUpdated: BN;
}

export interface AssetInfo {
  mint:           PublicKey;
  currentPct:     number;
  currentBase:    BN;
  thresholdUp:    number;
  thresholdDown:  number;
  feeMin:         number;
  feeMax:         number;
  currentFee:     number;
  maxPctMin:      number;
  maxPctMax:      number;
  allowed:        PublicKey[];
  isBlocked:      boolean;
  thresholdState: ThresholdState;
  layerStatus:    LayerConfirmation;
  pythData:       PythFeedData;
  pythFeedId:     number[];
  isStable:       boolean;
  staticFeeBps:   number;
}

export interface InfoPoolAccount {
  poolId:      PublicKey;
  authority:   PublicKey;
  assets:      AssetInfo[];
  poolSize:    BN;
  poolWeight:  BN;
  lastUpdated: BN;
  bump:        number;
}

// ── Routing types ────────────────────────────────────────────

export interface QuoteResult {
  pool:            PublicKey;
  amountOut:       BN;
  feeAmount:       BN;
  feeBps:          number;
  volumeConfirmed: boolean;
  allConfirmed:    boolean;
}

export interface RouteResult {
  bestPool:        PublicKey;
  expectedOut:     BN;
  feeBps:          number;
  priority:        number;
  poolWeight:      BN;
  volumeConfirmed: boolean;
  allConfirmed:    boolean;
}

// ── Computed fee claimable ────────────────────────────────────

export interface ClaimableResult {
  claimable: bigint;
  principal: bigint;
  poolFps:   bigint;
  feeDebt:   bigint;
  }
  
