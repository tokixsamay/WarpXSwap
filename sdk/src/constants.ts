import { PublicKey } from "@solana/web3.js";

export const GOVERNANCE_PROGRAM_ID = new PublicKey(
  "C1iFRYB3fw7Rq2i2JFruYLbJoGTxRb6ohYqerYBpUsLm"
);
export const POOL_PROGRAM_ID = new PublicKey(
  "4AXtXF5VWeWKLqP6vHKPpjoc7wQ8r4duDqZ4CENtzsqZ"
);
export const INFO_POOL_PROGRAM_ID = new PublicKey(
  "9MXoZpzQZzvURN1S1EARJLaDhFuGw3RAppQMYvGTcmPo"
);
export const ROUTING_PROGRAM_ID = new PublicKey(
  "3fdt9Skkj52bMvutU56CuBMZhrUsaStXBxGNtDPVCRSG"
);

export const GOVERNANCE_SEED = Buffer.from("governance");
export const PROPOSAL_SEED   = Buffer.from("proposal");
export const POOL_SEED       = Buffer.from("pool");
export const ASSET_SEED      = Buffer.from("asset");
export const INFO_POOL_SEED  = Buffer.from("info_pool");
