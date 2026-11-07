#!/usr/bin/env node
// `corepack pnpm tauri:smoke`: builds the app (debug profile, frontend embedded, no installer)
// and launches it with PF_SMOKE=1. The UI calls `app_ready` once it has rendered and reached the
// core; the app then exits 0. Fails on a non-zero exit or after 120 s.
//   --no-build   launch the existing build only (CI builds in its own step, for timings)
import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { root } from './lib/env.mjs';

const shell = process.platform === 'win32';
if (!process.argv.includes('--no-build')) {
  const r = spawnSync('corepack', ['pnpm', 'exec', 'tauri', 'build', '--debug', '--no-bundle'], {
    cwd: join(root, 'app'),
    stdio: 'inherit',
    shell,
  });
  if (r.status !== 0) process.exit(r.status ?? 1);
}

const exe = join(root, 'target', 'debug', shell ? 'pocket-forge.exe' : 'pocket-forge');
const tmp = mkdtempSync(join(tmpdir(), 'pf-smoke-'));
const env = {
  ...process.env,
  PF_SMOKE: '1',
  PF_DATA_DIR: join(tmp, 'data'),
  PF_LIBRARY_DIR: join(tmp, 'library'),
  PF_PROVIDER: 'fake',
};

console.log(`▶ launching ${exe}`);
const started = Date.now();
const child = spawn(exe, [], { env, stdio: ['ignore', 'pipe', 'inherit'] });
let out = '';
child.stdout.on('data', (d) => {
  out += d;
  process.stdout.write(d);
});
const timer = setTimeout(() => {
  console.error('::error::tauri:smoke timed out after 120 s');
  child.kill();
}, 120_000);

child.on('exit', (code) => {
  clearTimeout(timer);
  rmSync(tmp, { recursive: true, force: true });
  const ok = code === 0 && out.includes('PF_SMOKE ok');
  const secs = ((Date.now() - started) / 1000).toFixed(1);
  if (ok) console.log(`✔ smoke passed: the window loaded and reached the core in ${secs} s`);
  else console.error(`::error::tauri:smoke failed (exit ${code}) after ${secs} s`);
  process.exit(ok ? 0 : 1);
});
