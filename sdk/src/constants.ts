import { PublicKey } from "@solana/web3.js";

export const POOL_PROGRAM_ID     = new PublicKey("4AXtXF5VWeWKLqP6vHKPpjoc7wQ8r4duDqZ4CENtzsqZ");
export const INFO_POOL_PROGRAM_ID = new PublicKey("9MXoZpzQZzvURN1S1EARJLaDhFuGw3RAppQMYvGTcmPo");
export const GOVERNANCE_PROGRAM_ID = new PublicKey("C1iFRYB3fw7Rq2i2JFruYLbJoGTxRb6ohYqerYBpUsLm");
export const ROUTING_PROGRAM_ID   = new PublicKey("3fdt9Skkj52bMvutU56CuBMZhrUsaStXBxGNtDPVCRSG");

export const POOL_SEED       = Buffer.from("pool");
export const ASSET_SEED      = Buffer.from("asset");
export const LP_DEPOSIT_SEED = Buffer.from("lp_deposit");
export const INFO_POOL_SEED  = Buffer.from("info_pool");
export const ROUTER_SEED     = Buffer.from("router");

export const FEE_SCALE           = BigInt("1000000000"); // 1e9
export const BPS_DENOMINATOR     = BigInt("10000");
export const MAX_PCT_BUFFER      = 10;
export const MAX_FEE_BPS         = 500;
export const MIN_FEE_BPS         = 1;
export const CONFIDENCE_RATIO_BPS = 200; // 2% of price
export const MAX_BASE_SHIFT_BPS  = 100;  // 1% per confirmation cycle
export const FEE_SENSITIVITY     = 80;
