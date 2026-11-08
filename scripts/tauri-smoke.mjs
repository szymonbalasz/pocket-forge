#!/usr/bin/env node
// `corepack pnpm tauri:smoke`: builds the app (debug profile, frontend embedded, no installer)
// and runs two checks against it. Fails on the first failure or after 120 s per check.
//   1. Launch with PF_SMOKE=1. The UI calls `app_ready` once it has rendered and reached the
//      core; the app then exits 0.
//   2. Tile (Windows/WebView2 only, BL-07): launch again with the WebView2 debug port open,
//      connect with Playwright over CDP and, inside the page, open a committed fixture through
//      the core (`project_open` → `edt_tiles_open`) and fetch tile 0/0_0 through the `pf-tiles`
//      scheme (`/t/`), decoding it as the editor does. Proves the scheme, its registration and
//      the CSP end to end.
//   --no-build   launch the existing build only (CI builds in its own step, for timings)
import { spawn, spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { portFree, root } from './lib/env.mjs';
import { killTree } from './lib/proc.mjs';

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
const FIXTURE = join(root, 'fixtures', 'images', 'grid-map-512x640.png');
const TIMEOUT_MS = 120_000;

/** Launches the app with a fresh data and library folder. */
function launch(extraEnv) {
  const tmp = mkdtempSync(join(tmpdir(), 'pf-smoke-'));
  const library = join(tmp, 'library');
  mkdirSync(library, { recursive: true });
  const env = {
    ...process.env,
    PF_DATA_DIR: join(tmp, 'data'),
    PF_LIBRARY_DIR: library,
    PF_PROVIDER: 'fake',
    // Its own WebView2 profile: a second launch must not attach to the first one's browser
    // process (still shutting down, without the debug port), nor touch the real profile.
    WEBVIEW2_USER_DATA_FOLDER: join(tmp, 'webview2'),
    ...extraEnv,
  };
  console.log(`▶ launching ${exe}`);
  const child = spawn(exe, [], { env, stdio: ['ignore', 'pipe', 'inherit'] });
  let out = '';
  child.stdout.on('data', (d) => {
    out += d;
    process.stdout.write(d);
  });
  const exited = new Promise((resolve) => child.on('exit', (code) => resolve(code)));
  // WebView2 may hold its profile for a few seconds after the app exits.
  const cleanup = () => {
    for (let i = 0; ; i++) {
      try {
        rmSync(tmp, { recursive: true, force: true });
        return;
      } catch (e) {
        if (i >= 50) return console.warn(`  could not remove ${tmp}: ${e.message}`);
        Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 200);
      }
    }
  };
  return { child, exited, output: () => out, library, cleanup };
}

/** Command lines of the running WebView2 browser processes (diagnostics for a missing port). */
function webviewArgs() {
  const r = spawnSync(
    'powershell',
    [
      '-NoProfile',
      '-Command',
      "Get-CimInstance Win32_Process -Filter \"name='msedgewebview2.exe'\" | ? { $_.CommandLine -notlike '*--type=*' } | % { $_.CommandLine }",
    ],
    { encoding: 'utf8' },
  );
  return `WebView2 browser processes:
${r.stdout?.trim() || '(none)'}`;
}

const fail = (msg) => {
  console.error(`::error::tauri:smoke ${msg}`);
  process.exit(1);
};
const secs = (t) => ((Date.now() - t) / 1000).toFixed(1);

// ---- 1. the window loads and reaches the core
{
  const started = Date.now();
  const app = launch({ PF_SMOKE: '1' });
  const timer = setTimeout(() => {
    console.error('::error::tauri:smoke timed out after 120 s');
    app.child.kill();
  }, TIMEOUT_MS);
  const code = await app.exited;
  clearTimeout(timer);
  app.cleanup();
  if (!(code === 0 && app.output().includes('PF_SMOKE ok')))
    fail(`failed (exit ${code}) after ${secs(started)} s`);
  console.log(`✔ smoke passed: the window loaded and reached the core in ${secs(started)} s`);
}

// ---- 2. one tile loads through the pf-tiles scheme
if (process.platform !== 'win32') {
  console.log('… tile check skipped: it drives WebView2 over CDP (Windows only)');
  process.exit(0);
}
{
  const started = Date.now();
  let port = 9320;
  while (!(await portFree(port))) port++;
  const app = launch({ WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` });
  const image = join(app.library, 'smoke-map.png');
  copyFileSync(FIXTURE, image);
  let browser;
  let done = false;
  const finish = async (error) => {
    done = true;
    clearTimeout(timer);
    await browser?.close().catch(() => undefined);
    killTree(app.child);
    await app.exited;
    app.cleanup();
    if (error) fail(`tile check failed after ${secs(started)} s: ${error}`);
  };
  const timer = setTimeout(() => finish('timed out after 120 s'), TIMEOUT_MS);
  app.exited.then((code) => done || fail(`the app exited (${code}) during the tile check`));
  try {
    // The app's Playwright (already a dev dependency); CDP needs no browser download.
    const { chromium } = createRequire(join(root, 'app', 'package.json'))('@playwright/test');
    for (let i = 0; !browser; i++) {
      try {
        browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
      } catch (e) {
        if (i >= 120) throw new Error(`${e.message}
${webviewArgs()}`);
        await new Promise((r) => setTimeout(r, 500));
      }
    }
    let page;
    while (!(page = browser.contexts()[0]?.pages()[0])) await new Promise((r) => setTimeout(r, 200));
    await page.waitForFunction(() => typeof window.__TAURI_INTERNALS__?.invoke === 'function');
    const result = await page.evaluate(async (path) => {
      const invoke = (cmd, input) => window.__TAURI_INTERNALS__.invoke(cmd, { input });
      const project = await invoke('project_open', { path });
      const desc = await invoke('edt_tiles_open', {
        project_id: project.project_id,
        image_id: project.layers[0].id,
      });
      const lv = desc.levels[0];
      const url = `${desc.base_url}/${lv.level}/0_0.${lv.ext}`;
      // Level 0 may still be building: a missing tile is a 404 until it lands.
      for (let i = 0; ; i++) {
        const res = await fetch(url);
        if (res.ok) {
          const bitmap = await createImageBitmap(await res.blob());
          return {
            url,
            status: res.status,
            type: res.headers.get('content-type'),
            w: bitmap.width,
            h: bitmap.height,
            tile: Math.min(desc.tile_size, desc.width),
          };
        }
        if (i >= 100) return { url, status: res.status };
        await new Promise((r) => setTimeout(r, 200));
      }
    }, image);
    console.log(`  tile ${result.url} → ${result.status} ${result.type ?? ''} ${result.w ?? '?'}×${result.h ?? '?'}`);
    if (result.status !== 200 || !result.type?.startsWith('image/') || result.w !== result.tile)
      throw new Error(`tile 0/0_0 did not load: ${JSON.stringify(result)}`);
  } catch (e) {
    await finish(e.message);
  }
  await finish();
  console.log(`✔ smoke passed: tile 0/0_0 loaded through the pf-tiles scheme in ${secs(started)} s`);
}
