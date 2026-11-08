#!/usr/bin/env node
// `corepack pnpm tauri:smoke`: builds the app (debug profile, frontend embedded, no installer)
// and launches it with PF_SMOKE=1. The UI calls `app_ready` once it has rendered and reached the
// core. The app then exports the generated fixture map to PNG through the core commands
// (PF_SMOKE_EXPORT_IMAGE, export spec §13) and exits 0; the script decodes the exported file and
// checks it against the fixture. Fails on a non-zero exit, a failed check or after 120 s.
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
  PF_DATA_DIR: join(tmp, 'data'),
  PF_LIBRARY_DIR: join(tmp, 'library'),
  PF_PROVIDER: 'fake',
  PF_SMOKE_EXPORT_IMAGE: join(root, 'fixtures', 'images', 'grid-map-1024x1280.png'),
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
  rmSync(tmp, { recursive: true, force: true });
  const ok = code === 0 && out.includes('PF_SMOKE ok') && !exportError;
  const secs = ((Date.now() - started) / 1000).toFixed(1);
  if (ok)
    console.log(
      `✔ smoke passed: the window loaded, reached the core and exported the fixture map in ${secs} s`,
    );
  else if (exportError) console.error(`::error::tauri:smoke export check failed: ${exportError}`);
  else console.error(`::error::tauri:smoke failed (exit ${code}) after ${secs} s`);
  process.exit(ok ? 0 : 1);
});
