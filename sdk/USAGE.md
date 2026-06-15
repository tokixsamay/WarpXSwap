# @warpxswap/sdk — Usage Guide

## IDL Workflow

The WarpXSwap SDK **auto-loads** IDL files — no manual import required.

### Step 1: Build Anchor programs
```bash
anchor build
```
This generates the following files in `WarpXSwap/target/idl/`:
```
target/idl/pool_program.json
target/idl/info_pool_program.json
target/idl/governance_program.json
target/idl/routing_program.json
```

### Step 2: Commit IDL snapshots (optional but recommended)
```bash
pnpm --filter @warpxswap/sdk sync-idl
```
This copies IDL files into `sdk/idl/` — so teammates can use the SDK without running `anchor build`.

### Step 3: Check IDL paths
```bash
pnpm --filter @warpxswap/sdk inspect-idl
```
Output:
```
[idl] pool         → /path/to/target/idl/pool_program.json
[idl] infoPool     → /path/to/target/idl/info_pool_program.json
[idl] governance   → /path/to/target/idl/governance_program.json
[idl] routing      → /path/to/target/idl/routing_program.json
```

---

## Quick Start — All Programs

```typescript
import { AnchorProvider, Wallet } from "@coral-xyz/anchor";
import { Connection, Keypair } from "@solana/web3.js";
import { createPrograms, GovernanceClient } from "@warpxswap/sdk";

const connection = new Connection("http://localhost:8899", "confirmed");
const wallet     = new Wallet(Keypair.generate()); // your keypair
const provider   = new AnchorProvider(connection, wallet, {});

// All 4 programs load together — IDL is auto-detected
const { pool, infoPool, governance, routing } = createPrograms(provider);

// GovernanceClient
const poolOwner = wallet.publicKey;
const client    = new GovernanceClient(governance, poolOwner);
```

---

## Individual Programs

If you only need a single program:

```typescript
import { createPoolProgram, createGovernanceProgram } from "@warpxswap/sdk";

const poolProgram = createPoolProgram(provider);
const govProgram  = createGovernanceProgram(provider);
```

---

## Executing a Passed Proposal

Every `execute*` method returns `{ builder, accounts }`.
Use `.rpc()` to sign and send, or `.instruction()` to compose into a larger transaction.

### UpdateFeeRange
```typescript
const proposalId = new BN(3);
const executor   = wallet.publicKey;

const { builder } = await client.executeUpdateFeeRange(proposalId, {
  kind: "UpdateFeeRange",
  mint: new PublicKey("So11111111111111111111111111111111111111112"),
  newMin: 50,   // 0.5%
  newMax: 200,  // 2.0%
}, executor);

const sig = await builder.rpc();
console.log("Executed:", sig);
```

### UpdateThreshold
```typescript
const { builder } = await client.executeUpdateThreshold(proposalId, {
  kind:    "UpdateThreshold",
  mint:    new PublicKey("..."),
  newUp:   500, // 5.00%
  newDown: 300, // 3.00%
}, executor);

await builder.rpc();
```

### UpdateMaxPct
```typescript
const { builder } = await client.executeUpdateMaxPct(proposalId, {
  kind:   "UpdateMaxPct",
  mint:   new PublicKey("..."),
  newMin: 5,   // 5%
  newMax: 25,  // 25%
}, executor);

await builder.rpc();
```

### AddAsset
```typescript
const { builder } = await client.executeAddAsset(proposalId, {
  kind:          "AddAsset",
  mint:          new PublicKey("..."),
  maxPctMin:     5,
  maxPctMax:     25,
  feeMin:        10,
  feeMax:        100,
  thresholdUp:   500,
  thresholdDown: 300,
  initialBase:   new BN(0),
  allowed:       [],
}, executor);

await builder.rpc();
```

### RemoveAsset
```typescript
const { builder } = await client.executeRemoveAsset(proposalId, {
  kind: "RemoveAsset",
  mint: new PublicKey("..."),
}, executor);

await builder.rpc();
```

### UpdateAllowance
Allow SOL to receive USDC (trader provides USDC, receives SOL):
```typescript
const { builder } = await client.executeUpdateAllowance(proposalId, {
  kind:    "UpdateAllowance",
  asset:   solMint,   // updating the allowance list of asset_out (SOL)
  target:  usdcMint,  // allow or disallow USDC
  allowed: true,
}, executor);

await builder.rpc();
```

---

## Generic Dispatch

Iterate over a queue of passed proposals:

```typescript
const proposals = [
  { id: new BN(1), payload: { kind: "UpdateFeeRange", mint, newMin: 10, newMax: 80 } },
  { id: new BN(2), payload: { kind: "RemoveAsset", mint } },
];

for (const { id, payload } of proposals) {
  const { builder } = await client.executeProposal(id, payload as any, executor);
  await builder.rpc();
}
```

---

## PDA Helpers

```typescript
const [govPDA]      = client.getGovernancePDA(poolId);
const [proposalPDA] = client.getProposalPDA(poolId, new BN(5));
const [poolPDA]     = client.getPoolPDA();
const [assetPDA]    = client.getAssetPDA(mint);
const [infoPoolPDA] = client.getInfoPoolPDA(poolId);
```

---

## Composing Into a Larger Transaction

```typescript
const ix = await builder.instruction();
const tx = new Transaction().add(ix);
await provider.sendAndConfirm(tx);
```

---

## Custom Program IDs (devnet / testnet)

```typescript
const client = new GovernanceClient(governance, poolOwner, {
  poolProgramId:       new PublicKey("..."),
  infoProgramId:       new PublicKey("..."),
  governanceProgramId: new PublicKey("..."),
});
```

---

## IDL Search Order

The SDK looks for IDL files in this order:
1. `WarpXSwap/target/idl/` — authoritative source after `anchor build`
2. `WarpXSwap/sdk/idl/`    — committed copies after `pnpm sync-idl`

If not found in either location → an error is thrown:
```
IDL not found for 'pool'. Run 'anchor build' then 'pnpm sync-idl'.
```
