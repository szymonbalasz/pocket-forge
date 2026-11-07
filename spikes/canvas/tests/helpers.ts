import { chromium, type Browser, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
const __dirname = dirname(fileURLToPath(import.meta.url));

export const BASE = process.env.SPIKE_URL ?? 'http://localhost:5174';
export const LIBS = ['konva', 'fabric', 'pixi'] as const;

/** Fresh headed Edge (same Chromium engine as WebView2) per scenario, so memory numbers don't mix.
 *  vsync and the frame-rate cap are off: frame intervals show the real cost, not the display refresh. */
export async function launch(opts: { vsync?: boolean; highPerfGpu?: boolean } = {}): Promise<{ browser: Browser; page: Page }> {
  const args = [
    '--enable-precise-memory-info', '--disable-backgrounding-occluded-windows', '--disable-renderer-backgrounding',
    '--disable-background-timer-throttling', '--disable-features=CalculateNativeWinOcclusion', '--window-position=0,0',
  ];
  if (!opts.vsync) args.push('--disable-gpu-vsync', '--disable-frame-rate-limit');
  if (opts.highPerfGpu) args.push('--force_high_performance_gpu');
  const browser = await chromium.launch({ channel: 'msedge', headless: false, args });
  const ctx = await browser.newContext({ viewport: { width: 1640, height: 960 }, deviceScaleFactor: 1 });
  const page = await ctx.newPage();
  page.on('console', m => { if (m.type() === 'error') console.log('[page]', m.text()); });
  page.on('pageerror', e => console.log('[pageerror]', e.message));
  await page.goto(`${BASE}/?bench=1`);
  await page.waitForFunction(() => 'bench' in window);
  return { browser, page };
}

export function procMem(): Record<string, number> {
  if (process.platform !== "win32") return {};
  try {
    const out = execFileSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(__dirname, '..', 'scripts', 'proc-mem.ps1')], { encoding: 'utf8' });
    return JSON.parse(out.trim());
  } catch { return {}; }
}

export function save(name: string, data: unknown) {
  const dir = join(__dirname, '..', 'results', 'raw');
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, `${name}.json`), JSON.stringify(data, null, 1));
}
