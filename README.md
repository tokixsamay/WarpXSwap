# WarpXSwap — Adaptive Liquidity Protocol

> *The first AMM that feels the market pulse and protects LPs before damage happens.*
> *Not reactive. Not static. Proactive, adaptive, and manipulation-resistant by design.*

**Built on Solana · Powered by Pyth Network · Rust + Anchor**

---

## Table of Contents

- [Overview](#overview)
- [Core Innovations](#core-innovations)
- [Architecture](#architecture)
- [Repository Structure](#repository-structure)
- [Programs](#programs)
- [Crank (Off-Chain Keeper)](#crank-off-chain-keeper)
- [SDK](#sdk)
- [Tests](#tests)
- [Getting Started](#getting-started)
- [Deployment](#deployment)
- [Governance](#governance)
- [Security](#security)
- [Roadmap](#roadmap)

---

## Overview

WarpXSwap is a novel **Adaptive Market Maker (AMM)** protocol that fundamentally rethinks how liquidity pools protect LPs from Impermanent Loss (IL).

Every existing AMM — Uniswap, Raydium, Orca — reacts to price changes *after* damage is done. WarpXSwap acts **proactively**: it detects developing market conditions and adjusts pool behaviour *before* IL accumulates.

| Problem (Existing AMMs) | Solution (WarpXSwap) |
|---|---|
| Reactive: IL after damage done | Proactive: Rebalances before damage |
| Pair-based: Capital fragmented | Asset-based: One deposit, all interactions |
| No distinction: Growth vs pump | 3-signal confirmation: Growth vs pump |
| External arbitrage: Value leaves | Internal preference: Value stays |
| Per-asset fee silos: fees locked | Pool-wide accumulator: no fee ever lost |

---

## Core Innovations

### 1. Independent Asset Architecture

Assets in a WarpXSwap pool are **not pairs**. Each asset (SOL, BTC, ETH, USDC) is an independent entity with its own threshold, fee range, and concentration limit. A single pool can hold up to **10 assets** without fragmenting capital.

- Fee applies only to the **outgoing asset** on every swap.
- Each asset manages its own `allowed` list — which incoming assets it permits.
- Threshold triggers affect that asset's interactions only, not the whole pool.

### 2. 3-Layer Dynamic Threshold System

Distinguishes **genuine long-term growth** from **short-term pump manipulation** using three independent Pyth Network signals. All three must confirm before the threshold base shifts.

| Layer | Signal | Genuine Growth | Pump/Manipulation |
|---|---|---|---|
| 1 | TWAP (30 min / 4 hr / 24 hr EMA) | Sustained across all timeframes | Spike then drop |
| 2 | Volume trend | ≥10% increase vs prior window | Spike only, drops fast |
| 3 | Pyth Confidence Interval | Narrow (< 2% of price) | Wide (publishers disagree) |

When all three confirm → threshold base shifts gradually, **capped at 100 bps (1%) per cycle**.

### 3. V-Shape Range-Bound Fee System

LP sets `fee_min` and `fee_max` per asset. The protocol continuously adjusts within those bounds:

- **At base price (equilibrium):** `fee = fee_max` — maximum revenue.
- **As price deviates (either direction):** fee slides toward `fee_min` — incentivising arbitrageurs to restore balance.
- **At or beyond threshold:** `fee = fee_min` — maximum arbitrage incentive.

Stablecoin assets (USDC, USDT, PYUSD) use a flat `static_fee_bps` instead of the V-shape curve.

### 4. Pool-Wide Fee Accumulator

All swap fees — across every asset — flow into a single `pool_fps` accumulator. Every LP earns proportionally from **every swap**, regardless of which asset they deposited. No fee is ever lost or locked.

### 5. Access-Controlled Info Pool

Each pool has a dedicated `InfoPoolAccount` PDA writable only by the registered crank keypair. External wallets cannot write to it, preventing manipulation and enabling safe routing optimisation.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  WarpXSwap Protocol Stack                                       │
│                                                                 │
│  ┌────────────┐  ┌──────────────┐  ┌───────────┐  ┌─────────┐ │
│  │    Pool    │  │  Info Pool   │  │ Governance│  │ Routing │ │
│  │  Program   │  │   Program    │  │  Program  │  │ Program │ │
│  └─────┬──────┘  └──────┬───────┘  └─────┬─────┘  └────┬────┘ │
│        │    CPI ────────┘                 │             │      │
│        │                                  │             │      │
│  ┌─────▼──────────────────────────────────▼─────────────▼────┐ │
│  │                  Solana Runtime / PDAs                     │ │
│  └────────────────────────────────────────────────────────────┘ │
│                              ▲                                  │
│                    Pyth Network V2                              │
│              (Price · EMA/TWAP · Confidence)                    │
└─────────────────────────────────────────────────────────────────┘
                              ▲
                         Off-Chain Crank
                   (Keeper — runs every ~400ms)
```

**Crank sequence (per asset, per Solana slot):**
```
1. update_pyth_feeds        → Read Pyth PriceUpdateV2, update EMAs
2. push_oracle_price_to_pool → CPI → Pool: update asset.oracle_price
3. run_threshold_check      → 3-layer evaluation, may block/unblock inflow
4. calculate_and_push_fee   → Compute V-shape fee, CPI → Pool: update fee
```

---

## Repository Structure

```
WarpXSwap/
├── Anchor.toml                  # Workspace config, program IDs
├── package.json
│
├── programs/
│   ├── pool/                    # Core AMM — swaps, deposits, withdrawals
│   │   └── src/
│   │       ├── instructions/
│   │       │   ├── swap.rs
│   │       │   ├── deposit_withdraw.rs
│   │       │   ├── add_asset.rs
│   │       │   ├── allowance.rs
│   │       │   ├── governance_cpi.rs
│   │       │   ├── info_pool_cpi.rs
│   │       │   ├── initialize_pool.rs
│   │       │   └── read.rs
│   │       ├── state.rs
│   │       ├── constants.rs
│   │       ├── errors.rs
│   │       └── tests.rs
│   │
│   ├── info_pool/               # Oracle brain — fees, thresholds, Pyth
│   │   └── src/
│   │       ├── instructions/
│   │       │   ├── pyth.rs      # EMA/TWAP calculation
│   │       │   ├── threshold.rs # 3-layer confirmation system
│   │       │   ├── fee.rs       # V-shape fee calculation
│   │       │   ├── pool_metrics.rs
│   │       │   ├── governance.rs
│   │       │   └── read.rs
│   │       ├── utils.rs         # Fee math (on-chain)
│   │       ├── state.rs
│   │       └── constants.rs
│   │
│   ├── governance/              # On-chain governance — proposals, votes
│   │   └── src/
│   │       ├── instructions/
│   │       │   ├── proposal.rs
│   │       │   ├── vote.rs
│   │       │   ├── execute.rs
│   │       │   ├── finalize.rs
│   │       │   └── initialize.rs
│   │       └── state.rs
│   │
│   └── routing/                 # Smart routing — filter + priority engine
│       └── src/
│           ├── instructions/
│           │   ├── routing.rs   # Filter + priority algorithm
│           │   └── execute.rs
│           └── state.rs
│
├── crank/                       # Off-chain keeper (TypeScript)
│   └── src/
│       ├── index.ts
│       ├── loop.ts
│       ├── config.ts
│       ├── logger.ts
│       └── steps/
│           ├── updatePythFeeds.ts
│           ├── pushOraclePrice.ts
│           ├── runThresholdCheck.ts
│           ├── calculateAndPushFee.ts
│           └── pushVolume.ts
│
├── sdk/                         # TypeScript SDK for integrations
│   └── src/
│       ├── pool.ts
│       ├── infoPool.ts
│       ├── pdas.ts
│       ├── types.ts
│       └── constants.ts
│
├── tests/                       # Bankrun test suite
│   ├── pool-swap.test.ts
│   ├── pool-deposit-withdraw.test.ts
│   ├── pool-fee-accumulator.test.ts
│   ├── pool-weight.test.ts
│   ├── pool-oracle-staleness.test.ts
│   ├── pool-usd-fee-claim.test.ts
│   └── helpers/
│       ├── setup.ts
│       └── mint.ts
│
└── scripts/
    └── ci-check-replace-after-deploy.sh
```

---

## Programs

### Pool Program (`programs/pool`)

The core AMM program. Handles:

- **Swaps** — oracle-rate math, outgoing-only fee, max % concentration guard
- **Deposits / Withdrawals** — LP principal management, fee settlement
- **Fee Claims** — `claim_fees`, `compound_fees`, `public_exit`
- **Asset management** — `add_asset`, `set_allowance`
- **CPI receivers** — accepts oracle price and fee updates from Info Pool

**Key constants:**
```
FEE_SCALE           = 1_000_000_000  (1e9 — fee accumulator precision)
MAX_PCT_BUFFER      = 10             (hard cap = max_pct_max + 10%)
MIN_FEE_BPS         = 1              (0.01%)
MAX_FEE_BPS         = 500            (5.00%)
```

### Info Pool Program (`programs/info_pool`)

The oracle coordination brain. Handles:

- Pyth V2 price ingestion and EMA computation (3 timeframes)
- 3-Layer threshold evaluation
- V-shape fee calculation (`utils.rs`)
- CPI calls to Pool: `push_oracle_price`, `block_inflow`, `update_fee`

**Key constants:**
```
MAX_BASE_SHIFT_BPS      = 100   (1% max base shift per cycle)
CONFIDENCE_RATIO_BPS    = 200   (2% — Pyth publisher agreement threshold)
PYTH_MAX_STALENESS      = 10    (slots ≈ 4 seconds)
FEE_SENSITIVITY         = 80    (V-shape curve aggressiveness)
```

### Governance Program (`programs/governance`)

On-chain governance for pool parameter changes:

- **Proposers:** Top 10 LP holders by contribution
- **Voters:** All LP contributors, weighted by stake
- **Timelock:** Configurable per pool (≥86,400s on mainnet)
- **Cooldown:** 48 hours per proposer
- **Emergency proposals:** Require Top 10 majority before going public

**Proposal types:** `AddAsset`, `RemoveAsset`, `UpdateAllowance`, `UpdateMaxPct`, `UpdateThreshold`, `UpdateFeeRange`, `SetPythFeedId`, `SetInflowBlocked`

### Routing Program (`programs/routing`)

Two-stage routing engine:

**Stage 1 — Hard Filters (eliminate):**
1. `allowed` list check (outgoing asset must permit incoming)
2. `is_blocked` check (threshold not exceeded)
3. Pool liquidity ≥ `amount_out_required`
4. Pool liquidity ≥ `MIN_LIQUIDITY` (1,000,000 lamports)
5. `current_fee` ≤ user's `max_fee_bps` limit
6. `pool_is_active` (pool_size > 0)

**Stage 2 — Priority Sort (select best):**
1. Threshold pressure tier (P1 Exceeded > P2 Approaching ≥50% > P3 Neutral)
2. Lowest fee
3. `all_confirmed = true` first
4. Highest `pool_weight`

---

## Crank (Off-Chain Keeper)

The crank is the off-chain keeper that drives all live oracle/fee/threshold state.

### Setup

```bash
cd crank
npm install
cp .env.example .env
# Edit .env with your RPC URL, crank keypair path, and pool configs
```

### Configure pools in `.env`

```bash
POOL_CONFIGS='[{
  "poolOwner": "YOUR_POOL_AUTHORITY_PUBKEY",
  "assets": [
    {
      "mint":             "So11111111111111111111111111111111111111112",
      "mintAddr":         "So11111111111111111111111111111111111111112",
      "pythPriceAccount": "H6ARHf6YXhGYeQfUzQNGk6rDNnLBQKrenN712K4AQJEG"
    },
    {
      "mint":             "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
      "mintAddr":         "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
      "pythPriceAccount": "Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btez2ssa5GR3vTVE"
    }
  ]
}]'
```

### Run

```bash
# Development (hot reload)
npm run dev

# Production
npm run build && npm start
```

> **Important:** The crank keypair must match `InfoPool.authority` — the LP who initialized the InfoPool. Unauthorized keypairs are rejected on-chain.

### Pyth V2 Price Accounts (Devnet)

| Asset | Pyth PriceUpdateV2 Account |
|---|---|
| SOL/USD | `H6ARHf6YXhGYeQfUzQNGk6rDNnLBQKrenN712K4AQJEG` |
| BTC/USD | `GVXRSBjFk6e6J3NbVPXohDJetcTjaeeuykUpbQF8UoMU` |
| ETH/USD | `JBu1AL4obBcCMqKBBxhpWCNUt136ijcuMZLFvTP7iWdB` |
| USDC/USD | `Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btez2ssa5GR3vTVE` |

---

## SDK

TypeScript SDK for frontend and integration use.

```bash
cd sdk
npm install
```

```typescript
import { WarpXSwapPool, WarpXSwapInfoPool } from './sdk/src';

// Fetch pool state
const pool = await WarpXSwapPool.fetch(connection, poolPda);

// Fetch info pool state (fees, thresholds, oracle data)
const infoPool = await WarpXSwapInfoPool.fetch(connection, infoPoolPda);
```

---

## Tests

Bankrun-based TypeScript test suite — runs without a live validator (~10× faster than `solana-test-validator`).

### Prerequisites

```bash
# 1. Build programs (generates target/idl/*.json)
anchor build

# 2. Install test dependencies
cd tests
npm install
```

### Run Tests

```bash
# All tests (from workspace root)
anchor test

# Individual groups
npm run test:weight     # pool_weight accounting regressions
npm run test:fees       # pool-wide fps accumulator model
npm run test:swap       # swap instruction + oracle/fee/concentration math
npm run test:deposits   # deposit / withdraw / claim / compound flows
```

### Test Coverage

| File | What It Covers |
|---|---|
| `pool-swap.test.ts` | Oracle-rate math, outgoing-only fee, max % concentration guard, slippage, pool_fps after swap |
| `pool-deposit-withdraw.test.ts` | All deposit/withdraw/exit state invariants, partial exit, claim_fees, compound_fees |
| `pool-fee-accumulator.test.ts` | pool_fps formula, LP fee_debt, pending_fees on re-deposit, capital switching |
| `pool-weight.test.ts` | pool_weight bug fix regressions: starts at 0, deposit doesn't touch it, public_exit subtracts fee_share only |
| `pool-oracle-staleness.test.ts` | Behaviour when Pyth feed is stale or oracle price is unset |
| `pool-usd-fee-claim.test.ts` | USD-denominated fee claim flows, cross-asset fee accrual |

---

## Getting Started

### Prerequisites

- [Rust](https://rustup.rs/) — stable toolchain
- [Solana CLI](https://docs.solanalabs.com/cli/install) — v1.18+
- [Anchor CLI](https://www.anchor-lang.com/docs/installation) — v0.30.1
- Node.js v18+

### Install

```bash
git clone https://github.com/tokixsamay/WarpXSwap.git
cd WarpXSwap
npm install
```

### Build Programs

```bash
anchor build
```

### Run Tests

```bash
anchor test
```

---

## Deployment

### Devnet

```bash
# Deploy all programs
anchor deploy --provider.cluster devnet

# After deploy, update program IDs in Anchor.toml
ts-node scripts/ci-check-replace-after-deploy.sh
```

### Program IDs (Localnet)

| Program | ID |
|---|---|
| `pool_program` | `4AXtXF5VWeWKLqP6vHKPpjoc7wQ8r4duDqZ4CENtzsqZ` |
| `governance_program` | `C1iFRYB3fw7Rq2i2JFruYLbJoGTxRb6ohYqerYBpUsLm` |
| `info_pool_program` | `9MXoZpzQZzvURN1S1EARJLaDhFuGw3RAppQMYvGTcmPo` |
| `routing_program` | `3fdt9Skkj52bMvutU56CuBMZhrUsaStXBxGNtDPVCRSG` |

> Devnet and mainnet IDs are set to `REPLACE_AFTER_DEPLOY_*` placeholders — update after each deploy.

---

## Governance

Pool parameters are managed through on-chain governance:

```
Who proposes:  Top 10 LP holders by contribution
Who votes:     All LP contributors
Vote weight:   Proportional to stake_amount
Quorum:        min_votes_to_pass (set per pool at init)
Window:        48 hours
Timelock:      execute_delay_secs (≥86,400s on mainnet)
Cooldown:      48 hours per proposer
```

Emergency proposals require Top 10 majority approval before going to community vote.

---

## Security

- All arithmetic uses Rust's `checked_add`, `checked_mul`, `checked_div` — any overflow reverts.
- `asset.amount >= total_out` guard on every claim/exit prevents over-draw.
- Pyth staleness check: `get_price_no_older_than` — max 10 slots (~4 seconds).
- Info Pool writes restricted to registered crank keypair only.
- `is_blocked` flag rejects swap inflow when threshold is exceeded.
- 3 independent security audits required before mainnet launch.

---

## Roadmap

| Phase | Duration | Status |
|---|---|---|
| Phase 0 — Core Development | 0–4 months | ✅ Current |
| Phase 1 — Professional Code Review | 1.5 months | Upcoming |
| Phase 2 — Devnet / Testnet Deployment | 1 month | Upcoming |
| Phase 3 — Controlled Mainnet Launch | 1–2 months | Upcoming |
| Standalone Launch | After Phase 3 | — |

**Total estimated timeline: 12–14 months to standalone launch.**

---

## Links

- **X (Twitter):** [@TokiXSamay](https://x.com/TokiXSamay) — weekly progress reports
- **GitHub:** [github.com/tokixsamay/WarpXSwap](https://github.com/tokixsamay/WarpXSwap)
- **Whitepaper:** `WarpXSwap_Whitepaper_v5.md`

---

## License

All code will be open-sourced on standalone launch. Currently confidential — grant proposal stage.

---

*WarpXSwap · Adaptive Liquidity Protocol · Built on Solana · 2026*
