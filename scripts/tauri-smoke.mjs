#!/usr/bin/env node
// `corepack pnpm tauri:smoke`: builds the app (debug profile, frontend embedded, no installer)
// and launches it with PF_SMOKE=1 and PF_SMOKE_TILE_IMAGE (a committed fixture). Two steps, one
// launch (R46):
//   1. The UI renders, reaches the core and calls `app_ready` ("PF_SMOKE shell ok").
//   2. The app hands back the image; the UI creates a map project from it, opens it, and loads
//      tile 0/0_0 through the `pf-tiles` scheme as the editor does, then reports again
//      ("PF_SMOKE ok tile …") and the app exits 0 (BL-07).
//   3. Before exiting, the app exports PF_SMOKE_EXPORT_IMAGE to PNG through the core commands
//      (export spec §13, "PF_SMOKE export ok <file> <w>x<h> <bytes>"); the script decodes the file
//      and checks it against the fixture.
// Fails on a non-zero exit, a missing step, or after 120 s.
//   --no-build   launch the existing build only (CI builds in its own step, for timings)
import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { root } from './lib/env.mjs';
import { readPng } from './lib/png.mjs';

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
  PF_SMOKE_EXPORT_IMAGE: join(root, 'fixtures', 'images', 'grid-map-1024x1280.png'),
  // Its own WebView2 profile, so the smoke never touches the real one.
  WEBVIEW2_USER_DATA_FOLDER: join(tmp, 'webview2'),
};

/** The export step's output (`PF_SMOKE export ok <file> <w>x<h> <bytes>`) checked against the
 *  fixture: an IHDR/IDAT/IEND-only PNG (FR-EXP-10) that decodes pixel-equal (FR-EXP-04). */
function checkExport(text) {
  const m = /PF_SMOKE export ok (.+) (\d+)x(\d+) (\d+)\r?\n/.exec(text);
  if (!m) return 'the export step did not report ok';
  const [, file, w, h] = m;
  const got = readPng(file);
  const want = readPng(env.PF_SMOKE_EXPORT_IMAGE);
  if (got.width !== Number(w) || got.height !== Number(h))
    return `size ${got.width}x${got.height} ≠ reported ${w}x${h}`;
  if (got.width !== want.width || got.height !== want.height)
    return `size ${got.width}x${got.height} ≠ fixture ${want.width}x${want.height}`;
  const extra = got.chunks.filter((c) => !['IHDR', 'IDAT', 'IEND'].includes(c));
  if (extra.length) return `metadata chunks in the export: ${extra.join(', ')}`;
  if (!got.rgb.equals(want.rgb)) return 'the export does not decode pixel-equal to the fixture';
  return null;
}

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
  let exportError = null;
  try {
    exportError = checkExport(out);
  } catch (e) {
    exportError = String(e);
  }
  cleanup();
  const secs = ((Date.now() - started) / 1000).toFixed(1);
  const shellOk = out.includes('PF_SMOKE shell ok');
  const tileOk = /PF_SMOKE ok tile /.test(out);
  if (code === 0 && shellOk && tileOk && !exportError) {
    console.log(
      `✔ smoke passed: the window loaded, reached the core, loaded a tile and exported the fixture map in ${secs} s`,
    );
    process.exit(0);
  }
  const step = !shellOk
    ? 'the window never reached the core'
    : !tileOk
      ? 'the tile step failed'
      : `the export step failed: ${exportError}`;
  console.error(`::error::tauri:smoke failed (exit ${code}, ${step}) after ${secs} s`);
  process.exit(1);
});
