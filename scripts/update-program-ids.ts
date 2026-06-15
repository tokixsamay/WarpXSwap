#!/usr/bin/env ts-node
// ══════════════════════════════════════════════════════════════════
// update-program-ids — Auto-update program IDs after `anchor deploy`
//
// Reads keypairs from target/deploy/*.json
// Updates: Anchor.toml, each program's lib.rs, sdk/src/constants.ts
//
// Usage:
//   ts-node scripts/update-program-ids.ts [--network devnet|mainnet-beta]
//   ts-node scripts/update-program-ids.ts --dry-run
// ══════════════════════════════════════════════════════════════════

import * as fs   from "fs";
import * as path from "path";
import { Keypair } from "@solana/web3.js";

const ROOT      = path.resolve(__dirname, "..");
const DRY_RUN   = process.argv.includes("--dry-run");
const NETWORK   = (() => {
  const idx = process.argv.indexOf("--network");
  return idx !== -1 ? process.argv[idx + 1] : "devnet";
})();

// Programs to update
const PROGRAMS = [
  {
    key:         "pool_program",
    keypairFile: "pool_program-keypair.json",
    libRs:       "programs/pool/src/lib.rs",
    envKey:      "POOL_PROGRAM_ID",
    sdkExport:   "POOL_PROGRAM_ID",
  },
  {
    key:         "info_pool_program",
    keypairFile: "info_pool_program-keypair.json",
    libRs:       "programs/info_pool/src/lib.rs",
    envKey:      "INFO_POOL_PROGRAM_ID",
    sdkExport:   "INFO_POOL_PROGRAM_ID",
  },
  {
    key:         "governance_program",
    keypairFile: "governance_program-keypair.json",
    libRs:       "programs/governance/src/lib.rs",
    envKey:      "GOVERNANCE_PROGRAM_ID",
    sdkExport:   "GOVERNANCE_PROGRAM_ID",
  },
  {
    key:         "routing_program",
    keypairFile: "routing_program-keypair.json",
    libRs:       "programs/routing/src/lib.rs",
    envKey:      "ROUTING_PROGRAM_ID",
    sdkExport:   "ROUTING_PROGRAM_ID",
  },
];

// ── Helpers ───────────────────────────────────────────────────────

function readKeypair(filename: string): string | null {
  const full = path.join(ROOT, "target", "deploy", filename);
  if (!fs.existsSync(full)) {
    console.warn(`  ⚠  Keypair not found: ${full}`);
    return null;
  }
  const raw = JSON.parse(fs.readFileSync(full, "utf-8")) as number[];
  const kp  = Keypair.fromSecretKey(Uint8Array.from(raw));
  return kp.publicKey.toBase58();
}

function patch(filePath: string, oldContent: string, newContent: string, label: string) {
  if (DRY_RUN) {
    console.log(`  [dry-run] would patch ${label}`);
    return;
  }
  fs.writeFileSync(filePath, newContent, "utf-8");
  console.log(`  ✓  Patched ${label}`);
}

// ── Patch Anchor.toml ─────────────────────────────────────────────

function patchAnchorToml(ids: Record<string, string>) {
  const tomlPath = path.join(ROOT, "Anchor.toml");
  let content    = fs.readFileSync(tomlPath, "utf-8");
  const original = content;

  for (const [key, id] of Object.entries(ids)) {
    // Replace placeholder AND existing ID in the target network section
    const patterns = [
      new RegExp(`(\\[programs\\.${NETWORK}\\][\\s\\S]*?)${key}\\s*=\\s*"[^"]*"`, "m"),
    ];
    for (const re of patterns) {
      content = content.replace(re, (match, prefix) => `${prefix}${key} = "${id}"`);
    }
  }

  if (content !== original) {
    patch(tomlPath, original, content, `Anchor.toml [programs.${NETWORK}]`);
  } else {
    console.warn(`  ⚠  Anchor.toml section [programs.${NETWORK}] — no changes made`);
  }
}

// ── Patch lib.rs declare_id! ──────────────────────────────────────

function patchLibRs(libRsRel: string, newId: string) {
  const libRsPath = path.join(ROOT, libRsRel);
  if (!fs.existsSync(libRsPath)) {
    console.warn(`  ⚠  lib.rs not found: ${libRsPath}`);
    return;
  }

  const original = fs.readFileSync(libRsPath, "utf-8");
  const patched  = original.replace(
    /declare_id!\("([^"]+)"\)/,
    `declare_id!("${newId}")`
  );

  if (patched !== original) {
    patch(libRsPath, original, patched, libRsRel);
  } else {
    console.warn(`  ⚠  ${libRsRel} — declare_id not found or already correct`);
  }
}

// ── Patch sdk/src/constants.ts ────────────────────────────────────

function patchConstants(updates: { export: string; newId: string }[]) {
  const constPath = path.join(ROOT, "sdk", "src", "constants.ts");
  if (!fs.existsSync(constPath)) {
    console.warn(`  ⚠  constants.ts not found: ${constPath}`);
    return;
  }

  let content  = fs.readFileSync(constPath, "utf-8");
  const original = content;

  for (const { export: exp, newId } of updates) {
    content = content.replace(
      new RegExp(`(export const ${exp}\\s*=\\s*new PublicKey\\()\\s*"[^"]*"(\\s*\\))`),
      `$1"${newId}"$2`
    );
  }

  if (content !== original) {
    patch(constPath, original, content, "sdk/src/constants.ts");
  } else {
    console.warn("  ⚠  constants.ts — no changes made (exports may not match)");
  }
}

// ── Main ──────────────────────────────────────────────────────────

function main() {
  console.log(`\n🔑 update-program-ids — network: ${NETWORK}${DRY_RUN ? " [DRY RUN]" : ""}\n`);

  const ids: Record<string, string> = {};
  const constUpdates: { export: string; newId: string }[] = [];

  // Read all keypairs
  for (const prog of PROGRAMS) {
    const id = readKeypair(prog.keypairFile);
    if (!id) continue;

    ids[prog.key] = id;
    console.log(`  ${prog.key.padEnd(22)} → ${id}`);
  }

  if (Object.keys(ids).length === 0) {
    console.error("\n✗  No keypairs found in target/deploy/. Run `anchor build` first.\n");
    process.exit(1);
  }

  console.log("\nPatching files...\n");

  // 1. Anchor.toml
  patchAnchorToml(ids);

  // 2. Each lib.rs
  for (const prog of PROGRAMS) {
    if (ids[prog.key]) {
      patchLibRs(prog.libRs, ids[prog.key]);
      constUpdates.push({ export: prog.sdkExport, newId: ids[prog.key] });
    }
  }

  // 3. SDK constants
  patchConstants(constUpdates);

  console.log(`
✅ Done! Next steps:
   1. Run \`anchor build\` again to recompile with new program IDs
   2. Run \`anchor deploy --provider.cluster ${NETWORK}\`
   3. Run \`ts-node scripts/complete-setup.ts\`
   4. Run \`ts-node sdk/src/sync-idl.ts\` to update IDL snapshots
`);
}

main();
