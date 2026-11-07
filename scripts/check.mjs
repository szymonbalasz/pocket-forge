#!/usr/bin/env node
// `corepack pnpm check` (AGENTS §9): targeted local checks for what this branch changed.
//   UI changed   → tsc + ESLint + Prettier + Vitest
//   crate changed → rustfmt + clippy -D warnings + cargo test, for that crate only
//   Cargo files   → every crate + the dependency-direction check
// `--all` runs everything. CI always runs everything (AGENTS §5).
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';

import { root } from './lib/env.mjs';

const all = process.argv.includes('--all');
const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' });

function changedFiles() {
  let base = 'main';
  try {
    base = git('merge-base', 'HEAD', 'main').trim();
  } catch {
    // No main branch locally: compare with the working tree only.
  }
  const committed = git('diff', '--name-only', base, 'HEAD').split('\n');
  const working = git('status', '--porcelain', '--untracked-files=all')
    .split('\n')
    .map((l) => l.slice(3).replace(/^"|"$/g, '').split(' -> ').pop());
  return [...new Set([...committed, ...working].filter(Boolean))];
}

const CRATES = {
  'crates/pf-core/': 'pf-core',
  'crates/pf-imaging/': 'pf-imaging',
  'crates/pf-provider/': 'pf-provider',
  'app/src-tauri/': 'pocket-forge',
};
const RUST_SHARED = ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'clippy.toml', 'scripts/check-crate-deps.mjs'];
const UI_SHARED = ['package.json', 'pnpm-lock.yaml', 'pnpm-workspace.yaml'];

const files = changedFiles();
const crates = new Set();
let ui = all;
let rustShared = all;
for (const f of files) {
  const crate = Object.entries(CRATES).find(([dir]) => f.startsWith(dir));
  if (crate) crates.add(crate[1]);
  else if (f.startsWith('app/') || UI_SHARED.includes(f) || f.startsWith('fixtures/images/')) ui = true;
  if (RUST_SHARED.includes(f) || /(^|\/)Cargo\.toml$/.test(f)) rustShared = true;
}
if (rustShared) Object.values(CRATES).forEach((c) => crates.add(c));

const steps = [];
const step = (name, cmd, args, cwd = root) => steps.push({ name, cmd, args, cwd });
const app = join(root, 'app');

if (ui) {
  step('tsc', 'corepack', ['pnpm', 'exec', 'tsc', '--noEmit'], app);
  step('eslint', 'corepack', ['pnpm', 'exec', 'eslint', '--max-warnings', '0', '.'], app);
  step('prettier', 'corepack', ['pnpm', 'exec', 'prettier', '--check', '.'], app);
  step('vitest', 'corepack', ['pnpm', 'exec', 'vitest', 'run'], app);
}
if (crates.size) {
  step('rustfmt', 'cargo', ['fmt', '--all', '--check']);
  if (rustShared) step('crate deps', 'node', ['scripts/check-crate-deps.mjs']);
  if (crates.has('pocket-forge') && !existsSync(join(app, 'dist', 'index.html'))) {
    // tauri::generate_context! needs the built frontend.
    step('vite build', 'corepack', ['pnpm', 'exec', 'vite', 'build'], app);
  }
  for (const c of crates) {
    step(`clippy ${c}`, 'cargo', ['clippy', '-p', c, '--all-targets', '--', '-D', 'warnings']);
    step(`test ${c}`, 'cargo', ['test', '-p', c]);
  }
}

if (!steps.length) {
  console.log('check: nothing changed that has checks (use --all to run everything)');
  process.exit(0);
}

const failed = [];
for (const s of steps) {
  console.log(`\n▶ ${s.name}`);
  const r = spawnSync(s.cmd, s.args, { cwd: s.cwd, stdio: 'inherit', shell: process.platform === 'win32' });
  if (r.status !== 0) failed.push(s.name);
}
console.log(failed.length ? `\n✖ check failed: ${failed.join(', ')}` : `\n✔ check passed (${steps.length} steps)`);
process.exit(failed.length ? 1 : 0);
