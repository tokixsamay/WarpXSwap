#!/usr/bin/env ts-node
// ╔═══════════════════════════════════════════════════════════════════╗
// ║   WarpXSwap — COMPLETE END-TO-END SETUP                          ║
// ║                                                                   ║
// ║   Pool 1 — SOL / USDC  (Solana native)                           ║
// ║   Pool 2 — ETH / USDC  (Solana native)                           ║
// ║   Pool 3 — SOL / BTC   (Solana native)                           ║
// ║                                                                   ║
// ║   Phase 1: Pool initialization (3 pools + assets)                ║
// ║   Phase 2: InfoPool (Pyth oracle layer per pool)                  ║
// ║   Phase 3: Governance (one per pool)                              ║
// ║   Phase 4: Router registration (all 3 pools)                      ║
// ║   Phase 5: Stablecoin static fees (USDC → 0.03 %, skips V-shape) ║
// ╚═══════════════════════════════════════════════════════════════════╝
//
// Usage:
//   ts-node scripts/complete-setup.ts
//
// Env vars:
//   RPC_URL=http://127.0.0.1:8899
//   WALLET_PATH=~/.config/solana/id.json
//   SOL_MINT=So11111111111111111111111111111111111111112
//   ETH_MINT=<wrapped-eth-spl-mint>
//   USDC_MINT=<usdc-spl-mint>
//   BTC_MINT=<wrapped-btc-spl-mint>
//   DRY_RUN=false          (true = print plan without sending txs)

import * as fs     from "fs";
import * as os     from "os";
import * as path   from "path";
import * as crypto from "crypto";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import {
  getOrCreateAssociatedTokenAccount,
} from "@solana/spl-token";
import { AnchorProvider, Program, Wallet, BN } from "@coral-xyz/anchor";
import {
  POOL_PROGRAM_ID,
  GOVERNANCE_PROGRAM_ID,
  INFO_POOL_PROGRAM_ID,
  ROUTING_PROGRAM_ID,
  findPoolPDA,
  findInfoPoolPDA,
  findGovernancePDA,
  findAssetPDA,
} from "../sdk/src";

// ── CONSTANTS ────────────────────────────────────────────────────────

const RPC_URL = process.env.RPC_URL ?? "http://127.0.0.1:8899";
const DRY_RUN = (process.env.DRY_RUN ?? "false") === "true";

const WALLET_PATH = process.env.WALLET_PATH
  ? path.resolve(process.env.WALLET_PATH)
  : path.join(os.homedir(), ".config", "solana", "id.json");

const IDL_DIR = path.join(__dirname, "..", "target", "idl");

// Well-known mints (SPL token mints on Solana)
const SOL_MINT  = new PublicKey(process.env.SOL_MINT  ?? "So11111111111111111111111111111111111111112");
const ETH_MINT  = new PublicKey(process.env.ETH_MINT  ?? "7vfCXTUXx5WJV5JADk17DUJ4ksgau7utNKj4b963voxs");
const USDC_MINT = new PublicKey(process.env.USDC_MINT ?? "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
const BTC_MINT  = new PublicKey(process.env.BTC_MINT  ?? "9n4nbM75f5Ui33ZbPYXn59EwSgE8CGsHtAeTH5YFeJ9E");

// ── STABLECOIN STATIC FEE CONFIG ─────────────────────────────────────
// Mints that should use a fixed LP-set fee instead of the V-shape curve.
// Keys are base58 mint addresses; value is static_fee_bps (u16).
const STABLE_STATIC_FEE_BPS: Record<string, number> = {
  [new PublicKey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").toBase58()]: 3,  // USDC — 0.03 %
};

// ── PYTH FEED IDs ─────────────────────────────────────────────────────
// Official Pyth V2 feed IDs (hex-encoded 32 bytes).
// Source: https://pyth.network/price-feeds
const PYTH_FEED_ID: Record<string, string> = {
  [SOL_MINT.toBase58()]:  "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d",
  [ETH_MINT.toBase58()]:  "ff61491a931112ddf1bd8147cd1b641375f79f5825126d665480874634fd0ace",
  [USDC_MINT.toBase58()]: "eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a",
  [BTC_MINT.toBase58()]:  "e62df6c8b4a85fe1a67db44dc12de5db330f7ac66b72dc658afedf0f4a415b43",
};

// ── INITIAL BASE PRICES ───────────────────────────────────────────────
// Pyth prices use expo=-8: price = value_usd × 10^8
const INITIAL_BASE_PRICE: Record<string, BN> = {
  [SOL_MINT.toBase58()]:  new BN("15000000000"),   // $150
  [ETH_MINT.toBase58()]:  new BN("320000000000"),  // $3200
  [USDC_MINT.toBase58()]: new BN("100000000"),     // $1.00
  [BTC_MINT.toBase58()]:  new BN("6500000000000"), // $65000
};

// ── HELPER: load IDL ──────────────────────────────────────────────────
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function loadIdl(name: string): any {
  const p = path.join(IDL_DIR, `${name}.json`);
  if (!fs.existsSync(p)) {
    throw new Error(`IDL not found: ${p}\nRun 'anchor build' first.`);
  }
  return JSON.parse(fs.readFileSync(p, "utf-8"));
}

// ── HELPER: step banner ───────────────────────────────────────────────
let stepN = 0;
function step(title: string): void {
  stepN++;
  console.log(`\n┌─ Step ${stepN}: ${title}`);
}
function ok(msg: string): void {
  console.log(`│  ✓ ${msg}`);
}
function info(msg: string): void {
  console.log(`│  ℹ ${msg}`);
}
function done(): void {
  console.log("└──────────────────────────────────────────────────────");
}

// ── POOL TYPE ENUM ────────────────────────────────────────────────────
const PoolType = {
  Public:  { public:  {} },
  Private: { private: {} },
};

// ── MAIN ──────────────────────────────────────────────────────────────

async function main() {
  console.log("\n╔═══════════════════════════════════════════════════════╗");
  console.log("║   WarpXSwap — Complete End-to-End Setup               ║");
  console.log("╚═══════════════════════════════════════════════════════╝");
  console.log("   Pool 1: SOL/USDC  | Pool 2: ETH/USDC  | Pool 3: SOL/BTC");
  if (DRY_RUN) console.log("\n   ⚡ DRY RUN — no transactions will be sent\n");
  console.log();

  // ── Load payer ───────────────────────────────────────────────
  if (!fs.existsSync(WALLET_PATH)) throw new Error(`Wallet not found: ${WALLET_PATH}`);
  const rawKey = JSON.parse(fs.readFileSync(WALLET_PATH, "utf-8")) as number[];
  const payer  = Keypair.fromSecretKey(Uint8Array.from(rawKey));
  console.log(`RPC:    ${RPC_URL}`);
  console.log(`Payer:  ${payer.publicKey.toBase58()}`);

  const connection = new Connection(RPC_URL, "confirmed");
  const wallet     = new Wallet(payer);
  const provider   = new AnchorProvider(connection, wallet, {
    commitment: "confirmed",
    preflightCommitment: "confirmed",
  });

  // ── Load programs ────────────────────────────────────────────
  const poolProgram     = new Program(loadIdl("pool_program"),      provider);
  const infoPoolProgram = new Program(loadIdl("info_pool_program"),  provider);
  const govProgram      = new Program(loadIdl("governance"),         provider);
  const routerProgram   = new Program(loadIdl("routing"),            provider);

  // ══════════════════════════════════════════════════════════════
  //  PHASE 1 — Pool Initialization (3 pools)
  // ══════════════════════════════════════════════════════════════

  console.log("\n━━━ PHASE 1: Pool Initialization ━━━━━━━━━━━━━━━━━━━━━━━");

  // 3 LP keypairs — one per pool.
  const lpKeypairs = Array.from({ length: 3 }, (_, i) => {
    const seed = crypto.createHash("sha256")
      .update(`warpxswap-lp-${i}-${payer.publicKey.toBase58()}`)
      .digest();
    return Keypair.fromSeed(seed);
  });

  const poolDefs = [
    {
      label:  "POOL-1  SOL / USDC",
      lp:     lpKeypairs[0],
      type:   PoolType.Public,
      assets: [
        { mint: SOL_MINT,  label: "SOL",  feeMin: 10, feeMax: 100, threshUp: 200, threshDown: 120, maxMin: 10, maxMax: 80, isStable: false, staticFeeBps: 0 },
        // USDC is a stablecoin: thresholds must be 0; static fee used instead of V-shape curve.
        { mint: USDC_MINT, label: "USDC", feeMin: 3,  feeMax: 3,   threshUp: 0,   threshDown: 0,   maxMin: 20, maxMax: 80, isStable: true,  staticFeeBps: 3  },
      ],
    },
    {
      label:  "POOL-2  ETH / USDC",
      lp:     lpKeypairs[1],
      type:   PoolType.Public,
      assets: [
        { mint: ETH_MINT,  label: "ETH",  feeMin: 15, feeMax: 180, threshUp: 250, threshDown: 150, maxMin: 5,  maxMax: 60, isStable: false, staticFeeBps: 0 },
        { mint: USDC_MINT, label: "USDC", feeMin: 3,  feeMax: 3,   threshUp: 0,   threshDown: 0,   maxMin: 20, maxMax: 80, isStable: true,  staticFeeBps: 3 },
      ],
    },
    {
      label:  "POOL-3  SOL / BTC",
      lp:     lpKeypairs[2],
      type:   PoolType.Public,
      assets: [
        { mint: SOL_MINT, label: "SOL", feeMin: 20, feeMax: 200, threshUp: 300, threshDown: 180, maxMin: 5, maxMax: 60, isStable: false, staticFeeBps: 0 },
        { mint: BTC_MINT, label: "BTC", feeMin: 20, feeMax: 200, threshUp: 250, threshDown: 150, maxMin: 5, maxMax: 40, isStable: false, staticFeeBps: 0 },
      ],
    },
  ] as const;

  const poolPdas: PublicKey[] = [];

  for (const def of poolDefs) {
    step(`Initialize ${def.label}`);

    const [poolPda] = findPoolPDA(def.lp.publicKey, POOL_PROGRAM_ID);
    poolPdas.push(poolPda);
    info(`Pool PDA: ${poolPda.toBase58()}`);

    const poolExists = await connection.getAccountInfo(poolPda);
    if (poolExists) {
      ok("Pool already exists — skipping initialize");
    } else if (!DRY_RUN) {
      const tx = await poolProgram.methods
        .initializePool(def.type)
        .accounts({
          pool:          poolPda,
          baseAssetMint: def.assets[0].mint,
          authority:     def.lp.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([def.lp])
        .rpc();
      ok(`Pool initialized — tx: ${tx.slice(0, 20)}...`);
    } else {
      ok("[DRY] Would initialize pool");
    }

    // ── Add assets ──────────────────────────────────────────────
    for (const asset of def.assets) {
      const [assetPda] = findAssetPDA(poolPda, asset.mint, POOL_PROGRAM_ID);
      const assetExists = await connection.getAccountInfo(assetPda);

      if (assetExists) {
        info(`Asset ${asset.label} already added`);
        continue;
      }

      const otherMints = def.assets
        .filter(a => a.mint !== asset.mint)
        .map(a => a.mint);

      const basePrice = INITIAL_BASE_PRICE[asset.mint.toBase58()];
      if (!basePrice) throw new Error(
        `No INITIAL_BASE_PRICE for ${asset.label} (${asset.mint.toBase58()})`
      );

      const params = {
        mint:          asset.mint,
        feeMin:        asset.feeMin,
        feeMax:        asset.feeMax,
        thresholdUp:   asset.threshUp,
        thresholdDown: asset.threshDown,
        maxPctMin:     asset.maxMin,
        maxPctMax:     asset.maxMax,
        initialBase:   basePrice,
        allowed:       otherMints,
        isStable:      asset.isStable,
        staticFeeBps:  asset.staticFeeBps,
      };

      if (!DRY_RUN) {
        const tx = await poolProgram.methods
          .addAsset(params)
          .accounts({
            pool:          poolPda,
            asset:         assetPda,
            authority:     def.lp.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .signers([def.lp])
          .rpc();
        ok(`Asset ${asset.label} added — tx: ${tx.slice(0, 20)}...`);
      } else {
        ok(`[DRY] Would add asset ${asset.label}`);
      }
    }

    done();
  }

  // ══════════════════════════════════════════════════════════════
  //  PHASE 2 — InfoPool (shadow oracle layer for each pool)
  // ══════════════════════════════════════════════════════════════

  console.log("\n━━━ PHASE 2: InfoPool Initialization ━━━━━━━━━━━━━━━━━━━");

  for (let i = 0; i < poolDefs.length; i++) {
    const def      = poolDefs[i];
    const poolPda  = poolPdas[i];
    const [infoPda] = findInfoPoolPDA(poolPda, INFO_POOL_PROGRAM_ID);

    step(`InfoPool for ${def.label}`);
    info(`InfoPool PDA: ${infoPda.toBase58()}`);

    const infoExists = await connection.getAccountInfo(infoPda);
    if (infoExists) {
      ok("InfoPool already exists — skipping initialize");
    } else if (!DRY_RUN) {
      const tx = await infoPoolProgram.methods
        .initializeInfoPool(poolPda)
        .accounts({
          infoPool:      infoPda,
          authority:     def.lp.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([def.lp])
        .rpc();
      ok(`InfoPool initialized — tx: ${tx.slice(0, 20)}...`);
    } else {
      ok("[DRY] Would initialize InfoPool");
    }

    // ── Register assets + set Pyth feed IDs ──────────────────────
    for (const asset of def.assets) {
      const otherMints = def.assets
        .filter(a => a.mint !== asset.mint)
        .map(a => a.mint);

      const basePrice = INITIAL_BASE_PRICE[asset.mint.toBase58()];
      if (!basePrice) throw new Error(
        `No INITIAL_BASE_PRICE for ${asset.label}`
      );

      if (!DRY_RUN) {
        try {
          const tx = await infoPoolProgram.methods
            .governanceAddAsset(
              asset.mint,
              asset.maxMin,
              asset.maxMax,
              asset.feeMin,
              asset.feeMax,
              asset.threshUp,
              asset.threshDown,
              basePrice,
              otherMints,
              asset.isStable,
              asset.staticFeeBps,
            )
            .accounts({
              infoPool:            infoPda,
              governanceAuthority: def.lp.publicKey,
            })
            .signers([def.lp])
            .rpc();
          ok(`InfoPool: ${asset.label} registered — tx: ${tx.slice(0, 20)}...`);
        } catch (e: unknown) {
          const msg = e instanceof Error ? e.message : String(e);
          if (msg.includes("AlreadyInitialized")) {
            info(`InfoPool: ${asset.label} already registered`);
          } else {
            throw e;
          }
        }
      } else {
        ok(`[DRY] Would register InfoPool asset ${asset.label}`);
      }

      // Set Pyth V2 feed ID — required before crank can push prices
      const feedHex = PYTH_FEED_ID[asset.mint.toBase58()];
      if (!feedHex) throw new Error(
        `No PYTH_FEED_ID for ${asset.label} — add it to PYTH_FEED_ID map`
      );
      const feedIdBytes = Array.from(Buffer.from(feedHex, "hex")) as number[];

      if (!DRY_RUN) {
        const tx = await infoPoolProgram.methods
          .governanceSetPythFeedId(asset.mint, feedIdBytes)
          .accounts({
            infoPool:            infoPda,
            governanceAuthority: def.lp.publicKey,
          })
          .signers([def.lp])
          .rpc();
        ok(`InfoPool: Pyth feed set for ${asset.label} — tx: ${tx.slice(0, 20)}...`);
      } else {
        ok(`[DRY] Would set Pyth feed for ${asset.label}: ${feedHex.slice(0, 8)}...`);
      }
    }

    done();
  }

  // ══════════════════════════════════════════════════════════════
  //  PHASE 3 — Governance (one per pool)
  // ══════════════════════════════════════════════════════════════

  console.log("\n━━━ PHASE 3: Governance Initialization ━━━━━━━━━━━━━━━━━");

  const govPdas: PublicKey[] = [];

  for (let i = 0; i < poolDefs.length; i++) {
    const def      = poolDefs[i];
    const poolPda  = poolPdas[i];
    const [govPda]  = findGovernancePDA(poolPda, GOVERNANCE_PROGRAM_ID);
    govPdas.push(govPda);

    step(`Governance for ${def.label}`);
    info(`Gov PDA: ${govPda.toBase58()}`);

    const govExists = await connection.getAccountInfo(govPda);
    if (govExists) {
      ok("Governance already exists — skipping");
    } else if (!DRY_RUN) {
      const tx = await govProgram.methods
        .initializeGovernance(
          poolPda,          // pool_id: Pubkey — used as PDA seed
          new BN(1),        // min_votes_to_pass: 1 for devnet/testnet; raise for mainnet
          new BN(0),        // execute_delay_secs: 0 for devnet (immediate); set ≥ 86400 (24 h) for mainnet
        )
        .accounts({
          governance:    govPda,
          authority:     def.lp.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([def.lp])
        .rpc();
      ok(`Governance initialized — tx: ${tx.slice(0, 20)}...`);
    } else {
      ok("[DRY] Would initialize Governance");
    }

    done();
  }

  // ══════════════════════════════════════════════════════════════
  //  PHASE 4 — Router Registration (3 pools)
  // ══════════════════════════════════════════════════════════════

  console.log("\n━━━ PHASE 4: Router Registration ━━━━━━━━━━━━━━━━━━━━━━━");

  step("Initialize Router");
  const [routerPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("router"), payer.publicKey.toBuffer()],
    ROUTING_PROGRAM_ID,
  );
  info(`Router PDA: ${routerPda.toBase58()}`);

  const routerExists = await connection.getAccountInfo(routerPda);
  if (routerExists) {
    ok("Router already exists — skipping");
  } else if (!DRY_RUN) {
    const tx = await routerProgram.methods
      .initializeRouter()
      .accounts({
        router:        routerPda,
        authority:     payer.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
    ok(`Router initialized — tx: ${tx.slice(0, 20)}...`);
  } else {
    ok("[DRY] Would initialize Router");
  }
  done();

  step("Register all 3 pools with Router");
  for (let i = 0; i < poolPdas.length; i++) {
    const label = poolDefs[i].label;
    if (!DRY_RUN) {
      try {
        const tx = await routerProgram.methods
          .registerPool()
          .accounts({
            router:        routerPda,
            pool:          poolPdas[i],
            authority:     payer.publicKey,
            systemProgram: SystemProgram.programId,
          })
          .rpc();
        ok(`Pool ${i + 1}/3 registered (${label}) — tx: ${tx.slice(0, 20)}...`);
      } catch (e: unknown) {
        const msg = e instanceof Error ? e.message : String(e);
        if (msg.includes("already")) {
          info(`Pool ${i + 1}/3 already registered`);
        } else {
          throw e;
        }
      }
    } else {
      ok(`[DRY] Would register pool ${i + 1}/3 (${label})`);
    }
  }
  done();

  // ══════════════════════════════════════════════════════════════
  //  PHASE 5 — Stablecoin Configuration
  //  Mark stable assets (USDC, etc.) with a fixed LP fee so the
  //  crank skips the V-shape curve for them.  Uses the LP-auth
  //  bootstrap pattern (same signer as governanceAddAsset).
  // ══════════════════════════════════════════════════════════════

  console.log("\n━━━ PHASE 5: Stablecoin Static Fee Configuration ━━━━━━━━");

  for (let i = 0; i < poolDefs.length; i++) {
    const def     = poolDefs[i];
    const poolPda = poolPdas[i];
    const [infoPda] = findInfoPoolPDA(poolPda, INFO_POOL_PROGRAM_ID);

    for (const asset of def.assets) {
      const staticFeeBps = STABLE_STATIC_FEE_BPS[asset.mint.toBase58()];
      if (staticFeeBps === undefined) continue; // volatile — skip

      step(`Mark ${asset.label} as stable in ${def.label} (${staticFeeBps} bps)`);
      info(`InfoPool PDA: ${infoPda.toBase58()}`);
      info(`Mint:         ${asset.mint.toBase58()}`);
      info(`Static fee:   ${staticFeeBps} bps (${(staticFeeBps / 100).toFixed(2)} %)`);

      if (!DRY_RUN) {
        try {
          const tx = await infoPoolProgram.methods
            .governanceSetStable(asset.mint, true, staticFeeBps)
            .accounts({
              infoPool:            infoPda,
              governanceAuthority: def.lp.publicKey,
            })
            .signers([def.lp])
            .rpc();
          ok(`${asset.label} marked stable — tx: ${tx.slice(0, 20)}...`);
        } catch (e: unknown) {
          const msg = e instanceof Error ? e.message : String(e);
          // Idempotent — already stable is fine
          if (msg.includes("AlreadyInitialized") || msg.includes("already")) {
            info(`${asset.label} already marked stable`);
          } else {
            throw e;
          }
        }
      } else {
        ok(`[DRY] Would mark ${asset.label} stable at ${staticFeeBps} bps`);
      }

      done();
    }
  }

  // ══════════════════════════════════════════════════════════════
  //  FINAL SUMMARY
  // ══════════════════════════════════════════════════════════════

  const summary = {
    network:   DRY_RUN ? "DRY_RUN (devnet/localnet)" : `${RPC_URL}`,
    timestamp: new Date().toISOString(),

    pool1_sol_usdc: {
      label:   "SOL/USDC",
      poolPda: poolPdas[0].toBase58(),
      infoPda: findInfoPoolPDA(poolPdas[0], INFO_POOL_PROGRAM_ID)[0].toBase58(),
      govPda:  govPdas[0].toBase58(),
    },

    pool2_eth_usdc: {
      label:   "ETH/USDC",
      poolPda: poolPdas[1].toBase58(),
      infoPda: findInfoPoolPDA(poolPdas[1], INFO_POOL_PROGRAM_ID)[0].toBase58(),
      govPda:  govPdas[1].toBase58(),
    },

    pool3_sol_btc: {
      label:   "SOL/BTC",
      poolPda: poolPdas[2].toBase58(),
      infoPda: findInfoPoolPDA(poolPdas[2], INFO_POOL_PROGRAM_ID)[0].toBase58(),
      govPda:  govPdas[2].toBase58(),
    },

    router: {
      routerPda:       routerPda.toBase58(),
      poolsRegistered: 3,
    },

    stablecoins: Object.entries(STABLE_STATIC_FEE_BPS).map(([mint, bps]) => ({
      mint,
      staticFeeBps: bps,
      pct:          `${(bps / 100).toFixed(2)}%`,
    })),
  };

  const outputPath = path.join(__dirname, "..", "complete-setup.json");
  fs.writeFileSync(outputPath, JSON.stringify(summary, null, 2));

  console.log("\n");
  console.log("╔══════════════════════════════════════════════════════════════════╗");
  console.log("║                 WarpXSwap — Pool Architecture                   ║");
  console.log("╠══════════════════════════════════════════════════════════════════╣");
  console.log("║                                                                  ║");
  console.log("║  ┌────────────────────────────────────────────────────────┐     ║");
  console.log("║  │              ROUTER (routing program)                   │     ║");
  console.log("║  │  quote(in,out)    → best rate across all 3 pools       │     ║");
  console.log("║  │  execute_route(…) → CPI into best pool to swap         │     ║");
  console.log("║  └──────────┬──────────────────┬──────────────────┬───────┘     ║");
  console.log("║             │                  │                  │             ║");
  console.log("║      ┌──────▼──────┐    ┌──────▼──────┐    ┌──────▼──────┐    ║");
  console.log("║      │  POOL 1     │    │  POOL 2     │    │  POOL 3     │    ║");
  console.log("║      │  SOL/USDC   │    │  ETH/USDC   │    │  SOL/BTC    │    ║");
  console.log("║      └──────┬──────┘    └──────┬──────┘    └──────┬──────┘    ║");
  console.log("║             │                  │                  │             ║");
  console.log("║      ┌──────▼──────────────────▼──────────────────▼──────┐     ║");
  console.log("║      │     INFO POOL (Pyth oracle + 3-layer engine)       │     ║");
  console.log("║      │  Volatile: V-shape fee  │  Stable: static fee      │     ║");
  console.log("║      │  Pyth EMA TWAP → threshold check → fee CPI         │     ║");
  console.log("║      └────────────────────────────────────────────────────┘     ║");
  console.log("║                                                                  ║");
  console.log("║  Stablecoin fees (Phase 5)                                       ║");
  console.log("║    USDC → 3 bps (0.03 %)  fixed · no V-shape · de-peg active   ║");
  console.log("║                                                                  ║");
  console.log("╠══════════════════════════════════════════════════════════════════╣");
  console.log("║  NEXT STEPS                                                      ║");
  console.log("║                                                                  ║");
  console.log("║  1. Start the crank (Pyth price updates + threshold checks):    ║");
  console.log("║     ts-node scripts/crank.ts                                    ║");
  console.log("║                                                                  ║");
  console.log("║  2. Start the governance crank (auto-execute Passed proposals): ║");
  console.log("║     ts-node scripts/govern-crank.ts                             ║");
  console.log("║                                                                  ║");
  console.log("║  3. Try swaps via router:                                        ║");
  console.log("║     router.get_quote({ inMint: SOL_MINT, outMint: USDC_MINT })  ║");
  console.log("╚══════════════════════════════════════════════════════════════════╝\n");

  console.log(`Summary saved → ${outputPath}\n`);
}

main().catch((err) => {
  console.error("\n✗ Complete setup failed:", err.message ?? err);
  process.exit(1);
});
