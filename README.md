# WarpXSwap — Adaptive Liquidity Protocol

> **Built on Solana** · Rust + Anchor 0.30 · Pyth Oracle · 4 On-Chain Programs · TypeScript SDK

WarpXSwap is a next-generation AMM on Solana that solves Impermanent Loss through **proactive,
market-driven rebalancing**. Unlike existing AMMs that react after damage is done, WarpXSwap
continuously monitors asset prices and adjusts pool behaviour **before** thresholds are breached.

---

## The Problem WarpXSwap Solves

Existing Solana AMMs (Orca, Raydium, Meteora) all share one fundamental flaw: they react to
impermanent loss *after* it happens. By the time a price threshold is breached, LP capital is
already damaged and arbitrageurs have extracted value from the pool.

**WarpXSwap introduces proactive IL protection:**

- Continuously monitors 3 independent market signals per asset (TWAP alignment, volume trend,
  oracle confidence interval)
- Adjusts fees and blocks inflow *before* damage occurs — not after
- Distinguishes genuine long-term price growth from short-term manipulation, shifting the price
  base only when all 3 layers confirm a real market move
- LPs earn a fair, proportional share of swap fees via an O(1) accumulator — no snapshots,
  no iteration, no off-chain computation

This is not a fork of Uniswap or Raydium. The 3-layer confirmation engine, V-shape fee curve,
and single-asset accumulator model are original on-chain mechanisms built from scratch in Rust.

---

## Core Innovations

### 1 — Independent Asset Architecture
Pool assets are independent entities — not pairs. Each asset (SOL, BTC, ETH, USDC) has its own
threshold, fee range, max concentration limit, and allowed-interaction list. A single pool can hold
any number of assets without fragmenting capital or multiplying management complexity.

### 2 — 3-Layer Dynamic Threshold System (V1)
Uses Pyth Network data to distinguish **genuine long-term growth** from **short-term manipulation**:

| Layer | Source | Signal |
|-------|--------|--------|
| 1 | Pyth TWAP | 30 min + 4 hr + 24 hr TWAPs all aligned in same direction |
| 2 | DexScreener Volume | `volume_24h > volume_prev × 1.1` — rising trend, not spike |
| 3 | Pyth Confidence | Narrow interval = publishers agree = genuine signal |

All three layers confirmed simultaneously → threshold base **shifts** with real growth.
Any single layer unconfirmed → base stays fixed → manipulation resisted.
*(Layer 4 — internal buy/sell flow — deferred to V2.)*

### 3 — V-Shape Range-Bound Fee System
LP sets `fee_min` and `fee_max` per asset. The protocol continuously adjusts fee within those
bounds using a **symmetric V-shape curve** driven by distance from the oracle base price:

```
fee_max ──●───────────────────────────────────────────●── fee_max
           \                                         /
            \                                       /
fee_min ──────●─────────────────────────────────●──────── fee_min
          base − threshold_down           base + threshold_up
```

- **At base price:** fee = `fee_max` — pool balanced, maximum LP revenue
- **As deviation grows:** fee slides toward `fee_min` — arbitrageurs attracted to restore price
- **At threshold:** fee = `fee_min` — maximum incentive to correct imbalance
- **Asymmetric support:** `threshold_up` and `threshold_down` are independent per asset
- **Stablecoins:** bypass the curve entirely; use a fixed `static_fee_bps` set by governance

### 4 — Access-Controlled Info Pool
Shared Solana PDA acts as the network's coordination layer. Access is enforced at the smart
contract level: only authorised pool programs (verified by program ID) can write to the Info Pool.
External wallets and bots cannot interact with it directly. The Info Pool is readable by anyone
on-chain; it is **writable only by authorised pool programs** (not encrypted — access-controlled).

### 5 — Single-Asset Accumulator Fee Distribution
LP fee earnings are tracked with an O(1) accumulator model — no iteration over all positions on
every swap. Each asset's `fees_per_share` grows monotonically; each LP's `fee_debt` snapshots
the accumulator on deposit. Claimable fees at any time:

```
claimable = amount × (fees_per_share − fee_debt) / FEE_SCALE + pending_fees
```

`FEE_SCALE = 1e9`. Fees are distributed only to LPs who deposited the specific asset that was
swapped out — no cross-subsidisation between assets.

---

## Program IDs (Devnet / Localnet)

| Program | Address |
|---------|---------|
| Pool | `4AXtXF5VWeWKLqP6vHKPpjoc7wQ8r4duDqZ4CENtzsqZ` |
| Info Pool | `9MXoZpzQZzvURN1S1EARJLaDhFuGw3RAppQMYvGTcmPo` |
| Governance | `C1iFRYB3fw7Rq2i2JFruYLbJoGTxRb6ohYqerYBpUsLm` |
| Routing | `3fdt9Skkj52bMvutU56CuBMZhrUsaStXBxGNtDPVCRSG` |

---

## Architecture — 4 Programs

```
┌───────────────────────────────────────────────────────────────────┐
│                         TRADER / LP                               │
└────────────────────┬──────────────────────┬───────────────────────┘
                     │                      │
                     ▼                      ▼
          ┌─────────────────┐    ┌─────────────────────────┐
          │ ROUTING PROGRAM │    │     POOL PROGRAM         │
          │ find_best_pool  │───►│  swap · deposit          │
          │ get_quote       │    │  withdraw · claim_fees   │
          │ execute_route   │    │  compound_fees           │
          └─────────────────┘    └───────────┬─────────────┘
                                             │ CPI: oracle push,
                                             │ fee update, block signal,
                                             │ contributor register
                                             ▼
                                  ┌─────────────────────────┐
                                  │   INFO POOL PROGRAM      │
                                  │  Pyth price · 3-layer   │
                                  │  engine · V-shape fee   │
                                  │  threshold check         │
                                  └───────────┬─────────────┘
                                             │ governance CPI
                                             ▼
                                  ┌─────────────────────────┐
                                  │  GOVERNANCE PROGRAM      │
                                  │  proposals · voting     │
                                  │  timelock · execute     │
                                  └─────────────────────────┘
```

### CPI Data Flow (per crank tick, ~400 ms)

```
1. info_pool.update_pyth_feeds       → write Pyth price + TWAP + confidence to InfoPool
2. info_pool.push_oracle_price_to_pool → CPI → Pool: update oracle_price on AssetAccount
3. info_pool.run_threshold_check     → 3-layer engine → CPI → Pool: update fee + is_blocked
4. info_pool.calculate_and_push_fee  → V-shape calc → CPI → Pool: update current_fee

Every 60 s (DexScreener):
5. info_pool.push_volume             → write volume_24h; rotate prev window; enable layer 2
```

---

## Repository Structure

```
WarpXSwap/
├── Anchor.toml                       Anchor workspace config
├── Cargo.toml                        Rust workspace root
├── tsconfig.json                     TypeScript config for scripts + SDK
├── package.json                      ts-node, @coral-xyz/anchor, @solana/*
│
├── programs/
│   ├── pool/src/
│   │   ├── lib.rs                    Instruction dispatch
│   │   ├── state.rs                  PoolAccount · AssetAccount · LpDepositAccount
│   │   ├── constants.rs              FEE_SCALE · WEIGHT_PRECISION · seeds
│   │   ├── errors.rs                 PoolError enum
│   │   └── instructions/
│   │       ├── initialize_pool.rs    Pool PDA init
│   │       ├── add_asset.rs          AssetAccount init (governance-gated)
│   │       ├── swap.rs               AMM swap + fee accrual
│   │       ├── deposit_withdraw.rs   LP deposit · public_exit · claim_fees · compound_fees
│   │       ├── read.rs               View instructions (get_pool_state, get_asset_info)
│   │       ├── allowance.rs          governance_set_allowance
│   │       ├── info_pool_cpi.rs      update_oracle_price · update_fee · set_inflow_blocked
│   │       └── governance_cpi.rs     Governance CPI handlers (update params, add/remove asset)
│   │
│   ├── info_pool/src/
│   │   ├── lib.rs                    Instruction dispatch
│   │   ├── state.rs                  InfoPoolAccount · AssetInfo · PythFeedData · LayerConfirmation
│   │   ├── utils.rs                  V-shape fee calc · 3-layer check · should_block_inflow
│   │   ├── constants.rs              FEE_SENSITIVITY · MAX_CONFIDENCE_PCT · program IDs
│   │   ├── errors.rs                 InfoPoolError enum
│   │   └── instructions/
│   │       ├── initialize.rs         InfoPool PDA init · register_pool
│   │       ├── pyth.rs               update_pyth_feeds · push_volume · push_oracle_price_to_pool
│   │       ├── threshold.rs          run_threshold_check (3-layer engine + block CPI)
│   │       ├── fee.rs                calculate_and_push_fee (V-shape → Pool CPI)
│   │       ├── read.rs               get_threshold_state · get_pool_state (view)
│   │       └── governance.rs         governance handlers (threshold · fee range · max pct · assets · pyth feed)
│   │
│   ├── governance/src/
│   │   ├── lib.rs                    Instruction dispatch
│   │   ├── state.rs                  GovernanceAccount · ContributorAccount · ProposalAccount
│   │   ├── constants.rs              VOTING_WINDOW · QUORUM_BPS · BPS_DENOMINATOR · program IDs
│   │   ├── errors.rs                 GovernanceError enum
│   │   └── instructions/
│   │       ├── initialize.rs         Governance init · register_contributor · update_contributor_stake
│   │       ├── proposal.rs           create_proposal · approve_emergency
│   │       ├── vote.rs               cast_vote (quorum → Passed transition)
│   │       ├── finalize.rs           finalize_proposal (Active after deadline → Rejected)
│   │       └── execute.rs            execute_proposal (Governance PDA → CPI to Pool + InfoPool)
│   │
│   └── routing/src/
│       ├── lib.rs                    Instruction dispatch
│       ├── state.rs                  RouterAccount · RouteResult · QuoteResult
│       ├── constants.rs              MIN_LIQUIDITY · MAX_CANDIDATES · program IDs
│       ├── errors.rs                 RoutingError enum
│       └── instructions/
│           ├── initialize.rs         Router PDA init
│           ├── routing.rs            find_best_pool · get_quote (filter + priority)
│           └── execute.rs            execute_route (find + swap in one tx)
│
├── sdk/                              TypeScript SDK — use in dApps / scripts
│   ├── src/
│   │   ├── constants.ts             Program IDs + PDA seeds
│   │   ├── pda.ts                   PDA derivation helpers (findPoolPDA, findGovernancePDA, …)
│   │   ├── types.ts                 Shared TypeScript types
│   │   ├── idl-loader.ts            IDL JSON loader (requires anchor build)
│   │   ├── client.ts                GovernanceClient
│   │   ├── pool-client.ts           PoolClient (swap, deposit, withdraw, claim_fees)
│   │   ├── info-pool-client.ts      InfoPoolClient (fee info, threshold state)
│   │   ├── routing-client.ts        RoutingClient (quote, execute_route)
│   │   └── pool-setup.ts            PoolSetupClient (end-to-end pool creation helper)
│   └── USAGE.md
│
└── scripts/                          Off-chain operator tools (ts-node)
    ├── complete-setup.ts             End-to-end setup: all 3 pools + InfoPools + Governance
    ├── crank.ts                      Main keeper: Pyth updates + fee push + threshold check (~400 ms)
    ├── govern-crank.ts               Governance keeper: poll proposals → auto-execute Passed ones (~30 s)
    ├── propose-pyth-feed.ts          CLI: stage a SetPythFeedId governance proposal
    ├── test-validate-payload.ts      Regression test: boundary-value proposal creation checks
    ├── devnet-setup.ts               Devnet-specific initialisation helper
    ├── setup-pool.ts                 Single-pool setup helper
    ├── deploy.ts                     Program deployment helper
    ├── update-program-ids.ts         Patch program IDs across constants.rs files
    └── verify-deploy.ts              Post-deploy health check
```

---

## Governance

```
Who can propose:    Top 10 LP contributors by deposit stake
Who can vote:       All registered LP contributors
Vote weight:        Equal — 1 per contributor
Quorum:             min_votes_to_pass cast + 51% YES (both required)
Voting window:      48 hours
Proposer cooldown:  48 hours per wallet
Timelock:           execute_delay_secs per governance (0 on devnet; ≥ 24 h on mainnet)
Execution:          Anyone can call execute_proposal after timelock elapses

Emergency proposals:
  Top 10 majority (≥ 6 of 10) approve before going to community vote
```

### Proposal Types (8 total)

| # | Type | Effect |
|---|------|--------|
| 1 | `AddAsset` | Init AssetAccount (Pool) + AssetInfo (InfoPool) atomically |
| 2 | `RemoveAsset` | Close AssetAccount + remove AssetInfo (requires zero balance) |
| 3 | `UpdateFeeRange` | Set `fee_min` / `fee_max` on Pool + InfoPool |
| 4 | `UpdateThreshold` | Set `threshold_up` / `threshold_down` on Pool + InfoPool |
| 5 | `UpdateMaxPct` | Set `max_pct_min` / `max_pct_max` on Pool + InfoPool |
| 6 | `UpdateAllowance` | Add/remove a mint from an asset's allowed-interaction list |
| 7 | `SetPythFeedId` | Rotate the 32-byte Pyth V2 feed ID for an asset |
| 8 | `SetInflowBlocked` | Manually block or unblock inflow (emergency circuit-breaker) |

All proposals that affect both Pool and InfoPool use a **single atomic transaction** via
Governance PDA signer seeds — both succeed or both revert.

### Proposal Lifecycle

```
create_proposal
      │
      ├─ Normal → Active (48h voting window opens immediately)
      │
      └─ Emergency → PendingEmergencyApproval
                           │
                     ≥ 6 Top-10 approvals
                           │
                         Active (48h window opens)

Active ──────────────────────────────► finalize_proposal → Rejected
  │  (voting window closes, <51% yes)
  │
  │ 51% yes votes cast during window
  ▼
Passed ──► [timelock: execute_delay_secs] ──► execute_proposal ──► Executed
```

---

## LP Fee Operations

| Instruction | Description |
|-------------|-------------|
| `deposit` | Add principal; snapshot `fees_per_share` into `fee_debt`; register with Governance |
| `public_exit` | Withdraw principal + all accrued fees in one instruction |
| `claim_fees` | Collect accrued fees to wallet; stay in pool; reset `fee_debt` |
| `compound_fees` | Re-invest accrued fees as new principal; no token transfer; reset `fee_debt` |

Fee formula per asset per LP:
```
claimable = amount × (fees_per_share − fee_debt) / 1_000_000_000 + pending_fees
```

---

## Routing — Filter + Priority

**Stage 1 — Hard Filters (any fails → pool eliminated):**

1. `asset_in` in `asset_out.allowed` list
2. `asset_out.is_blocked == false` (not threshold-exceeded)
3. Pool liquidity ≥ `amount_in`
4. Pool liquidity ≥ `MIN_LIQUIDITY` (1,000,000 lamports)
5. `asset_out.current_fee ≤ user max_fee_bps`
6. `pool.is_active` (pool_size > 0)

**Stage 2 — Priority Sort (among remaining pools):**

```
Tier 1 (P1): ExceededUp or ExceededDown       — most urgently needs arbitrage
Tier 2 (P2): Approaching ≥ 50% of threshold   — moderately needs balancing
Tier 3 (P3): Neutral                           — standard selection

Within each tier, sort by:
  1. Lowest fee_bps
  2. all_confirmed = true first (all 3 layers confirmed)
  3. Highest pool_weight (deeper liquidity)
```

---

## Crank Operations

Two off-chain keeper processes keep pools live:

### Main Crank (`scripts/crank.ts`) — ~400 ms interval

```
Per tick:
  update_pyth_feeds(mint)              → Hermes API → write price/TWAP/confidence to InfoPool
  push_oracle_price_to_pool(mint)      → InfoPool CPI → Pool oracle_price update
  run_threshold_check(mint)            → 3-layer engine → fee recalc → block/unblock CPI
  calculate_and_push_fee(mint)         → V-shape calc → Pool current_fee update

Per minute:
  push_volume(mint, volume_24h)        → DexScreener (free, no API key) → InfoPool volume update
                                         Triggers base-price window rotation (prev ← current)
```

### Governance Crank (`scripts/govern-crank.ts`) — ~30 s interval

```
For each pool:
  Fetch all proposals with status = Passed
  Check: now ≥ proposal.execute_after (timelock elapsed)
  Call execute_proposal → Pool + InfoPool atomic update
  Log: ProposalExecuted event
```

---

## Technology Stack

| Component | Technology |
|-----------|-----------|
| Blockchain | Solana (mainnet / devnet / localnet) |
| Smart Contracts | Rust · Anchor 0.30 |
| Runtime | Node.js 24 · TypeScript 5 |
| Price Oracle | Pyth Network V2 (Hermes API · PriceUpdateV2 accounts) |
| Volume Data | DexScreener REST API (free · no API key) |
| Info Pool State | Solana PDA — access-controlled write (CPI-only) |
| Fee Distribution | Single-asset accumulator (O(1) · FEE_SCALE = 1e9) |
| SDK | TypeScript (`sdk/`) — GovernanceClient · PoolClient · InfoPoolClient · RoutingClient |
| Token Standard | SPL Token |

---

## Setup & Operation

### Prerequisites

```bash
# Solana CLI + Anchor 0.30 + Node.js 24 + pnpm
anchor --version   # 0.30.x
solana --version   # 1.18.x
node --version     # v24.x
```

### Build

```bash
cd WarpXSwap
anchor build          # compiles all 4 programs; generates target/idl/*.json
```

### Full Setup (local validator)

```bash
# Terminal 1 — local validator
solana-test-validator

# Terminal 2 — deploy + initialise all 3 pools
ts-node scripts/complete-setup.ts

# Terminal 3 — main crank (keeps prices/fees/thresholds live)
ts-node scripts/crank.ts

# Terminal 4 — governance crank (auto-executes passed proposals)
ts-node scripts/govern-crank.ts
```

### Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------| 
| `RPC_URL` | `http://127.0.0.1:8899` | Solana RPC endpoint |
| `WALLET_PATH` | `~/.config/solana/id.json` | Operator keypair |
| `CRANK_INTERVAL_MS` | `400` | Main crank tick interval |
| `VOLUME_REFRESH_MS` | `60000` | DexScreener volume refresh interval |
| `SOL_MINT` | `So111…` | SOL wrapped mint |
| `ETH_MINT` | `7vfC…` | Wrapped ETH SPL mint |
| `USDC_MINT` | `EPjF…` | USDC SPL mint |
| `BTC_MINT` | `9n4n…` | Wrapped BTC SPL mint |

### One-Shot Operator Tools

```bash
# Stage a SetPythFeedId governance proposal (rotate Pyth feed for an asset)
ts-node scripts/propose-pyth-feed.ts \
  --pool <pool-PDA>    \
  --mint <token-mint>  \
  --feed <32-byte-hex> \
  [--emergency]        \
  [--dry-run]

# Validate proposal creation boundary conditions (regression test)
ts-node scripts/test-validate-payload.ts --pool <pool-PDA>

# Verify all programs deployed and initialised correctly
ts-node scripts/verify-deploy.ts
```

---

## Key Design Decisions

### Why V-Shape (Not Piecewise Linear)?
The V-shape curve is a single continuous formula — no segment lookup, minimal compute cost,
and symmetric behaviour in both directions. The curve steepness is controlled by `FEE_SENSITIVITY`
(on-chain constant = 80), making it easy to audit and adjust via governance.

### Why Accumulator Fee Distribution (Not Snapshot)?
O(1) per-swap accounting — pool size has no effect on compute cost. New LPs cannot retroactively
claim fees from before their deposit (fair entry). Existing LPs lose no earnings on re-deposit
(`pending_fees` locks accrue
