# WarpXSwap — Pre-Deployment Simulation

## Purpose

Before deploying WarpXSwap's on-chain programs, a full frontend simulation of the protocol was built to validate core logic — swap execution, dynamic fee curves, multi-asset liquidity pools, and governance — in a realistic, interactive environment. This simulation was built using Replit and lives in this `simulation/` folder of the repo.

The goal was to stress-test protocol mechanics end-to-end *before* committing them to Anchor programs and deploying to devnet/mainnet, so that design flaws could be caught early when they're cheap to fix.

## Why Simulate First

WarpXSwap's core innovation — proactive LP protection via a dynamic threshold and fee system — depends on several interacting mechanisms (oracle price tracking, V-shaped fee curves, pool weight rebalancing, governance-adjustable parameters). These are hard to reason about purely on paper. A working simulation lets the actual user-facing behavior be observed, iterated on, and validated against intended design before locking it into on-chain code.

## What This Simulation Covers

- **Swap execution** — simulated swap flow between independent pool assets (SOL, USDC, USDT, PYUSD), including slippage tolerance selection and price impact display.
- **Liquidity provisioning** — deposit/withdraw flow for a multi-asset public pool, LP share accounting, and per-LP position tracking (pool share %, USD value).
- **Dynamic fee system (V-shape)** — fee curve behavior as live price moves away from a threshold/base price, with configurable threshold percentages and a hard cap on max fee.
- **Per-asset pool analytics** — independent tracking per asset of pool balance, USD value, share of TVL, 24h volume, and fees earned, reflecting WarpXSwap's independent-asset (non-pair-based) architecture.
- **Governance** — proposal creation and voting flow for governable pool parameters (e.g. max asset concentration, price thresholds), including quorum/participation tracking and pass thresholds.
- **Wallet state separation** — distinct Trader Wallet vs LP Wallet views to mirror how a real user would interact with trading vs. liquidity-provider roles.

## Status

This is a **frontend simulation only** — it uses simulated pool execution and does not interact with deployed on-chain programs. It was used purely to validate UX flow and protocol logic assumptions ahead of implementing and auditing the actual Rust/Anchor programs.

## Relationship to On-Chain Programs

Logic validated here informed (and was cross-checked against) the actual implementation in the four on-chain programs (`pool`, `info_pool`, `governance`, `routing`). Any divergence between simulated behavior and on-chain program behavior should be treated as a bug to investigate, not as the on-chain program being "wrong by default."
