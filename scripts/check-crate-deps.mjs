#!/usr/bin/env node
// Enforces the crate dependency direction (AGENTS §9): app → pf-core → pf-imaging, pf-provider, pf-chat.
// Reads `cargo metadata`; no extra tool. Run: corepack pnpm check:crates
import { execFileSync } from 'node:child_process';

/** Workspace crate → the workspace crates it may depend on (normal and build deps). */
const ALLOWED = {
  'pocket-forge': ['pf-core'],
  'pf-core': ['pf-imaging', 'pf-provider', 'pf-chat'],
  'pf-imaging': [],
  'pf-provider': [],
  'pf-chat': [],
};
/** Crates that must never depend on Tauri. */
const NO_TAURI = ['pf-core', 'pf-imaging', 'pf-provider', 'pf-chat'];

const meta = JSON.parse(
  execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], {
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  }),
);
const members = new Set(meta.packages.map((p) => p.name));
const errors = [];

for (const pkg of meta.packages) {
  const allowed = ALLOWED[pkg.name];
  if (!allowed) {
    errors.push(`${pkg.name}: new workspace crate; add it to ALLOWED in scripts/check-crate-deps.mjs`);
    continue;
  }
  for (const dep of pkg.dependencies) {
    if (dep.kind === 'dev') continue;
    if (members.has(dep.name) && !allowed.includes(dep.name)) {
      errors.push(`${pkg.name} → ${dep.name} breaks the direction app → pf-core → pf-imaging, pf-provider, pf-chat`);
    }
    if (NO_TAURI.includes(pkg.name) && dep.name.startsWith('tauri')) {
      errors.push(`${pkg.name} → ${dep.name}: domain crates have no Tauri dependency`);
    }
  }
}

if (errors.length) {
  for (const e of errors) console.error(`::error::${e}`);
  process.exit(1);
}
console.log(`Crate dependency direction OK (${[...members].join(', ')})`);
