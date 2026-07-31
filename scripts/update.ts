// Deploy every file in tools/ to the bin folder (byte-for-byte copy).
//
// If a bin copy differs AND is newer than the repo copy, it was probably
// edited in place in bin; it is skipped with a warning so the edit is not
// lost. Re-run with --force to overwrite anyway (after salvaging the edit).
//
// bun run update [--force]     env: JUSTTOOLS_BIN overrides the target dir

import { readdirSync, readFileSync, statSync, copyFileSync, existsSync } from "node:fs";
import { join } from "node:path";

const BIN = process.env.JUSTTOOLS_BIN ?? "C:\\cmd\\bin";
const TOOLS = join(import.meta.dir, "..", "tools");
const force = process.argv.includes("--force");

if (!existsSync(BIN)) {
  console.error(`update: bin folder not found: ${BIN}`);
  process.exit(1);
}

let updated = 0;
let unchanged = 0;
let skipped = 0;

for (const name of readdirSync(TOOLS).sort()) {
  const src = join(TOOLS, name);
  const dst = join(BIN, name);
  if (!statSync(src).isFile()) continue;

  const srcBytes = readFileSync(src);
  if (existsSync(dst)) {
    const dstBytes = readFileSync(dst);
    if (srcBytes.equals(dstBytes)) {
      unchanged++;
      continue;
    }
    if (!force && statSync(dst).mtimeMs > statSync(src).mtimeMs) {
      console.warn(
        `!! ${name}: bin copy differs and is NEWER than the repo copy; skipped.\n` +
        `   Copy the bin edit into tools/ first, or re-run with --force to overwrite.`,
      );
      skipped++;
      continue;
    }
  }
  copyFileSync(src, dst);
  console.log(`>> ${name} -> ${dst}`);
  updated++;
}

console.log(`update: ${updated} updated, ${unchanged} unchanged${skipped ? `, ${skipped} SKIPPED` : ""} (${BIN})`);
if (skipped) process.exit(1);
