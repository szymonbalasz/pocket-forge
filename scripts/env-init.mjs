#!/usr/bin/env node
// Claims a dev slot for this worktree and writes .env.local (AGENTS §9 per-worktree isolation).
//   Vite dev port = 5200 + 10 × slot, e2e port = dev port + 1.
//   PF_DATA_DIR / PF_LIBRARY_DIR point inside the worktree (.pf-dev/, gitignored).
// Claims are recorded in <tmp>/pocket-forge/slots/ so two worktrees never share a slot.
// Re-running keeps the slot; it fails fast if the slot's port is taken. --force re-picks.
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { envFile, portFree, portOf, readEnvLocal, root } from './lib/env.mjs';

const MAX_SLOTS = 20;
const force = process.argv.includes('--force');
const claims = join(tmpdir(), 'pocket-forge', 'slots');
mkdirSync(claims, { recursive: true });

/** slot → worktree path, dropping claims whose worktree is gone. */
function claimed() {
  const out = new Map();
  for (const f of readdirSync(claims)) {
    const file = join(claims, f);
    try {
      const { slot, worktree } = JSON.parse(readFileSync(file, 'utf8'));
      if (existsSync(worktree)) out.set(slot, worktree);
      else rmSync(file);
    } catch {
      rmSync(file, { force: true });
    }
  }
  return out;
}

const current = readEnvLocal();
const taken = claimed();
let slot = null;

if (current.PF_SLOT && !force) {
  slot = Number(current.PF_SLOT);
  if (!(await portFree(portOf(slot)))) {
    console.error(
      `Port ${portOf(slot)} (slot ${slot}) is in use. Stop the process using it, or re-pick: corepack pnpm env:init -- --force`,
    );
    process.exit(1);
  }
} else {
  for (let s = 0; s < MAX_SLOTS; s++) {
    const owner = taken.get(s);
    if (owner && owner !== root) continue;
    if (await portFree(portOf(s))) {
      slot = s;
      break;
    }
  }
  if (slot === null) {
    console.error(`No free slot among ${MAX_SLOTS}: every port 5200–${portOf(MAX_SLOTS - 1)} is claimed or in use.`);
    process.exit(1);
  }
}

for (const [s, wt] of taken) if (wt === root && s !== slot) rmSync(join(claims, `${s}.json`), { force: true });
writeFileSync(join(claims, `${slot}.json`), JSON.stringify({ slot, worktree: root }));

const dev = join(root, '.pf-dev');
const dataDir = join(dev, 'data');
const libraryDir = join(dev, 'library');
mkdirSync(dataDir, { recursive: true });
mkdirSync(libraryDir, { recursive: true });

writeFileSync(
  envFile,
  [
    '# Written by `corepack pnpm env:init` (AGENTS §9). Per worktree, gitignored: never commit.',
    `PF_SLOT=${slot}`,
    `PF_VITE_PORT=${portOf(slot)}`,
    `PF_E2E_PORT=${portOf(slot) + 1}`,
    `PF_DATA_DIR=${dataDir}`,
    `PF_LIBRARY_DIR=${libraryDir}`,
    '',
  ].join('\n'),
);
console.log(`Slot ${slot}: dev port ${portOf(slot)}, e2e port ${portOf(slot) + 1}, data and library in .pf-dev/`);
