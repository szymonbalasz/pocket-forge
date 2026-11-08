#!/usr/bin/env node
// `corepack pnpm tauri:smoke`: builds the app (debug profile, frontend embedded, no installer)
// and launches it with PF_SMOKE=1 and PF_SMOKE_TILE_IMAGE (a committed fixture). Two steps, one
// launch (R46):
//   1. The UI renders, reaches the core and calls `app_ready` ("PF_SMOKE shell ok").
//   2. The app hands back the image; the UI creates a map project from it, opens it, and loads
//      tile 0/0_0 through the `pf-tiles` scheme as the editor does, then reports again
//      ("PF_SMOKE ok tile …") and the app exits 0 (BL-07).
// Fails on a non-zero exit, a missing step, or after 120 s.
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
  PF_SMOKE_TILE_IMAGE: join(root, 'fixtures', 'images', 'grid-map-512x640.png'),
  PF_DATA_DIR: join(tmp, 'data'),
  PF_LIBRARY_DIR: join(tmp, 'library'),
  PF_PROVIDER: 'fake',
  // Its own WebView2 profile, so the smoke never touches the real one.
  WEBVIEW2_USER_DATA_FOLDER: join(tmp, 'webview2'),
};

/** WebView2 may hold its profile for a few seconds after the app exits. */
function cleanup() {
  for (let i = 0; ; i++) {
    try {
      rmSync(tmp, { recursive: true, force: true });
      return;
    } catch (e) {
      if (i >= 50) return console.warn(`  could not remove ${tmp}: ${e.message}`);
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 200);
    }
  }
}

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
  cleanup();
  const secs = ((Date.now() - started) / 1000).toFixed(1);
  const shellOk = out.includes('PF_SMOKE shell ok');
  const tileOk = /PF_SMOKE ok tile /.test(out);
  if (code === 0 && shellOk && tileOk) {
    console.log(`✔ smoke passed: the window loaded, reached the core and loaded a tile in ${secs} s`);
    process.exit(0);
  }
  const step = !shellOk ? 'the window never reached the core' : 'the tile step failed';
  console.error(`::error::tauri:smoke failed (exit ${code}, ${step}) after ${secs} s`);
  process.exit(1);
});
