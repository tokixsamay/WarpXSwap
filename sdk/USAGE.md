# @warpxswap/sdk — Usage Guide

## IDL Workflow

WarpXSwap SDK IDL files ko **auto-load** karta hai — manually import karne ki zaroorat nahi.

### Step 1: Anchor programs build karo
```bash
anchor build
```
Yeh `WarpXSwap/target/idl/` mein yeh files banata hai:
```
target/idl/pool_program.json
target/idl/info_pool_program.json
target/idl/governance_program.json
target/idl/routing_program.json
```

### Step 2: IDL snapshots commit karo (optional lekin recommended)
```bash
pnpm --filter @warpxswap/sdk sync-idl
```
Yeh IDL files ko `sdk/idl/` mein copy karta hai — taaki teammates bina `anchor build` ke SDK use kar sakein.

### Step 3: IDL paths check karo
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
const wallet     = new Wallet(Keypair.generate()); // aapka keypair
const provider   = new AnchorProvider(connection, wallet, {});

// Saare 4 programs ek saath load hote hain — IDL auto-detect hota hai
const { pool, infoPool, governance, routing } = createPrograms(provider);

// GovernanceClient
const poolOwner = wallet.publicKey;
const client    = new GovernanceClient(governance, poolOwner);
```

---

## Individual Programs

Agar sirf ek program chahiye:

```typescript
import { createPoolProgram, createGovernanceProgram } from "@warpxswap/sdk";

const poolProgram = createPoolProgram(provider);
const govProgram  = createGovernanceProgram(provider);
```

---

## Executing a Passed Proposal

Har `execute*` method `{ builder, accounts }` return karta hai.
`.rpc()` se sign+send karo, `.instruction()` se larger transaction mein compose karo.

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
SOL ko USDC allow karo (trader USDC de sakta hai, SOL milega):
```typescript
const { builder } = await client.executeUpdateAllowance(proposalId, {
  kind:    "UpdateAllowance",
  asset:   solMint,   // asset_out (SOL) ki allowance list update ho rahi hai
  target:  usdcMint,  // USDC ko allow/disallow karo
  allowed: true,
}, executor);

await builder.rpc();
```

---

## Generic Dispatch

Passed proposals ki queue iterate karo:

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

SDK IDL dhundhta hai is order mein:
1. `WarpXSwap/target/idl/` — `anchor build` ke baad (authoritative)
2. `WarpXSwap/sdk/idl/`    — `pnpm sync-idl` ke baad committed copies

Agar dono jagah nahi mila → error throw hoga:
```
IDL not found for 'pool'. Run 'anchor build' then 'pnpm sync-idl'.
```
