# WarpXSwap InfoPool Crank

The crank is the off-chain keeper that drives all live oracle/fee/threshold state into the WarpXSwap InfoPool and Pool programs. It must run continuously (every Solana slot, ~400 ms) to keep swap prices and fees accurate.

## Crank 4-Step Sequence (per asset, per slot)

For every pool and every registered asset, the crank fires these four instructions in order:

```
1. update_pyth_feeds        — Read Pyth V2 PriceUpdateV2 account.
                              Updates InfoPool EMAs (twap_short/medium/long),
                              confidence interval, and spot price.

2. push_oracle_price_to_pool — CPI from InfoPool → Pool.
                              Writes InfoPool's spot price into
                              AssetAccount.oracle_price so swap.rs has
                              the freshest price without a circular CPI dep.

3. run_threshold_check      — 3-layer evaluation:
                              Layer 1: TWAP alignment (short > medium > long)
                              Layer 2: Volume trend (volume_24h >= volume_prev × 1.10)
                              Layer 3: Confidence ≤ price × 2%
                              When all 3 confirm → base shifts ≤ 100 bps.
                              May CPI to Pool: block_inflow / unblock_inflow.

4. calculate_and_push_fee   — Compute new fee (V-shape or static for stablecoins)
                              and CPI to Pool if fee changed.
```

### Volume push (separate loop, ~60 s cadence)

```
push_volume — Fetches 24h trading volume from DexScreener and pushes it
              into InfoPool.PythFeedData.volume_24h.
              Rotates: volume_prev ← volume_24h, then writes new value.
```

## Setup

### 1. Install dependencies

```bash
cd WarpXSwap/crank
npm install
```

### 2. Configure environment

```bash
cp .env.example .env
# Edit .env with your RPC URL, crank keypair, and pool configs
```

### 3. Configure pools

Set `POOL_CONFIGS` in `.env` as a JSON array:

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

### 4. Build and run

```bash
# Development (ts-node, hot reload)
npm run dev

# Production (compile then run)
npm run build && npm start
```

## Authority requirement

The crank keypair must match `InfoPool.authority` — the LP who initialized the InfoPool. The on-chain `crank` constraint (`has_one = authority`) was fixed in the bug fix pass to prevent unauthorized callers from running `calculate_and_push_fee`.

## Pyth V2 price accounts (devnet)

| Asset | Pyth PriceUpdateV2 Account |
|-------|---------------------------|
| SOL/USD | `H6ARHf6YXhGYeQfUzQNGk6rDNnLBQKrenN712K4AQJEG` |
| BTC/USD | `GVXRSBjFk6e6J3NbVPXohDJetcTjaeeuykUpbQF8UoMU` |
| ETH/USD | `JBu1AL4obBcCMqKBBxhpWCNUt136ijcuMZLFvTP7iWdB` |
| USDC/USD | `Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btez2ssa5GR3vTVE` |
