#!/usr/bin/env ts-node
/**
 * sync-idl — copy IDL JSON files from target/idl/ → sdk/idl/
 *            copy TS types from target/types/ → sdk/idl/types/
 *
 * Run after `anchor build`:
 *   pnpm --filter @warpxswap/sdk sync-idl
 *
 * This lets you commit the IDL snapshots so teammates and CI
 * can use the SDK without running anchor build themselves.
 */

import * as fs from "fs";
import * as path from "path";

const ROOT       = path.resolve(__dirname, "../..");  // WarpXSwap/
const TARGET_IDL = path.join(ROOT, "target", "idl");
const TARGET_TYP = path.join(ROOT, "target", "types");
const SDK_IDL    = path.join(ROOT, "sdk", "idl");
const SDK_TYP    = path.join(SDK_IDL, "types");

const PROGRAMS = [
  "pool_program",
  "info_pool_program",
  "governance_program",
  "routing_program",
];

function ensureDir(dir: string) {
  if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
}

function copyIfExists(src: string, dest: string, label: string) {
  if (fs.existsSync(src)) {
    fs.copyFileSync(src, dest);
    console.log(`  ✓  ${label}`);
  } else {
    console.warn(`  ✗  ${label}  (not found — skipped)`);
  }
}

function main() {
  ensureDir(SDK_IDL);
  ensureDir(SDK_TYP);

  console.log(`\nSyncing IDLs from:\n  ${TARGET_IDL}\n  → ${SDK_IDL}\n`);

  for (const name of PROGRAMS) {
    // IDL JSON
    copyIfExists(
      path.join(TARGET_IDL, `${name}.json`),
      path.join(SDK_IDL,    `${name}.json`),
      `${name}.json`
    );
    // TypeScript types (optional)
    copyIfExists(
      path.join(TARGET_TYP, `${name}.ts`),
      path.join(SDK_TYP,    `${name}.ts`),
      `types/${name}.ts`
    );
  }

  console.log("\nDone. Commit sdk/idl/ to keep IDL snapshots in source control.\n");
}

main();
