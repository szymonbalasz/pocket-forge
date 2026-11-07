// Tree-shaken bundle cost of each adapter (library + our thin adapter), minified + gzip.
// Usage: node scripts/bundle-size.mjs
import { build } from 'vite';
import { readdirSync, readFileSync, rmSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const out = join(root, 'dist-size');
const res = {};
for (const lib of ['konva', 'fabric', 'pixi']) {
  rmSync(out, { recursive: true, force: true });
  await build({
    root, logLevel: 'silent', configFile: false, publicDir: false,
    build: { outDir: out, minify: true, lib: { entry: join(root, `src/adapters/${lib}.ts`), formats: ['es'], fileName: lib } },
  });
  let raw = 0, gz = 0;
  for (const f of readdirSync(out)) { const b = readFileSync(join(out, f)); raw += b.length; gz += gzipSync(b, { level: 9 }).length; }
  res[lib] = { minKB: Math.round(raw / 1024), gzipKB: Math.round(gz / 1024) };
}
rmSync(out, { recursive: true, force: true });
console.log(JSON.stringify(res));
