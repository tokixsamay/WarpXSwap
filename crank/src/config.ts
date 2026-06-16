import * as dotenv from "dotenv";
import { Keypair, Connection, clusterApiUrl } from "@solana/web3.js";
import bs58 from "bs58";

dotenv.config();

function requireEnv(key: string): string {
  const val = process.env[key];
  if (!val) throw new Error(`Missing required env var: ${key}`);
  return val;
}

export interface CrankConfig {
  connection:           Connection;
  crankKeypair:         Keypair;
  poolProgramId:        string;
  infoPoolProgramId:    string;
  rpcUrl:               string;
  slotIntervalMs:       number;
  volumeIntervalMs:     number;
  maxRetries:           number;
  retryDelayMs:         number;
  dexscreenerBaseUrl:   string;
  logLevel:             "debug" | "info" | "warn" | "error";
}

export function loadConfig(): CrankConfig {
  const rpcUrl = process.env["RPC_URL"] ?? clusterApiUrl("devnet");
  const connection = new Connection(rpcUrl, {
    commitment:           "confirmed",
    confirmTransactionInitialTimeout: 60_000,
  });

  let crankKeypair: Keypair;
  const privateKey = process.env["CRANK_PRIVATE_KEY"];
  if (privateKey) {
    try {
      const decoded = bs58.decode(privateKey);
      crankKeypair = Keypair.fromSecretKey(decoded);
    } catch {
      const bytes = JSON.parse(privateKey) as number[];
      crankKeypair = Keypair.fromSecretKey(Uint8Array.from(bytes));
    }
  } else {
    crankKeypair = Keypair.generate();
    console.warn("[crank] CRANK_PRIVATE_KEY not set — using ephemeral keypair (devnet only)");
  }

  return {
    connection,
    crankKeypair,
    poolProgramId:     process.env["POOL_PROGRAM_ID"]      ?? "4AXtXF5VWeWKLqP6vHKPpjoc7wQ8r4duDqZ4CENtzsqZ",
    infoPoolProgramId: process.env["INFO_POOL_PROGRAM_ID"] ?? "9MXoZpzQZzvURN1S1EARJLaDhFuGw3RAppQMYvGTcmPo",
    rpcUrl,
    slotIntervalMs:    Number(process.env["SLOT_INTERVAL_MS"]   ?? "400"),
    volumeIntervalMs:  Number(process.env["VOLUME_INTERVAL_MS"] ?? "60000"),
    maxRetries:        Number(process.env["MAX_RETRIES"]        ?? "3"),
    retryDelayMs:      Number(process.env["RETRY_DELAY_MS"]     ?? "500"),
    dexscreenerBaseUrl: process.env["DEXSCREENER_URL"] ?? "https://api.dexscreener.com/latest/dex",
    logLevel:          (process.env["LOG_LEVEL"] ?? "info") as CrankConfig["logLevel"],
  };
                                           }
    
