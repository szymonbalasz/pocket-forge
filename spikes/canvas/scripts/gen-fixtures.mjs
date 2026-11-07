// Procedural synthetic battlemap fixtures (no real art). Deterministic (seeded).
// Usage: node scripts/gen-fixtures.mjs [sizes...]   e.g. node scripts/gen-fixtures.mjs 8192 16384
// Output (gitignored): public/fixtures/
//   map-<n>.jpg            single image
//   tiles-<n>/manifest.json + tiles-<n>/<level>/<col>_<row>.jpg   512 px DeepZoom-style pyramid
//   patch.png              1024 px patch for the layer test
import sharp from 'sharp';
import { mkdirSync, writeFileSync, rmSync, renameSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'public', 'fixtures');
mkdirSync(outDir, { recursive: true });
const sizes = process.argv.slice(2).map(Number).filter(Boolean);
if (!sizes.length) sizes.push(8192, 16384);

const CELL = 100; // px per grid square (#11: 70–140)
const TILE = 512;

function mulberry32(a) {
  return () => {
    a |= 0; a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// 1024² tileable fBm value-noise texture, sampled with wrap.
const NT = 1024;
function makeNoise(seed) {
  const rnd = mulberry32(seed);
  const out = new Float32Array(NT * NT);
  for (let oct = 0, amp = 0.5, period = 128; oct < 5; oct++, amp *= 0.5, period /= 2) {
    const g = NT / period;
    const lat = new Float32Array(g * g).map(() => rnd());
    for (let y = 0; y < NT; y++) {
      const fy = y / period, y0 = Math.floor(fy) % g, y1 = (y0 + 1) % g, ty = fy - Math.floor(fy);
      const sy = ty * ty * (3 - 2 * ty);
      for (let x = 0; x < NT; x++) {
        const fx = x / period, x0 = Math.floor(fx) % g, x1 = (x0 + 1) % g, tx = fx - Math.floor(fx);
        const sx = tx * tx * (3 - 2 * tx);
        const a = lat[y0 * g + x0], b = lat[y0 * g + x1], c = lat[y1 * g + x0], d = lat[y1 * g + x1];
        out[y * NT + x] += amp * ((a + (b - a) * sx) * (1 - sy) + (c + (d - c) * sx) * sy);
      }
    }
  }
  return out;
}

// Cell map: 0 rock, 1 floor, 2 wall, 3 corridor, 4 water, 5 pillar-room floor
function makeCells(cols, rows, seed) {
  const rnd = mulberry32(seed);
  const cells = new Uint8Array(cols * rows);
  const rooms = [];
  const target = Math.floor((cols * rows) / 140);
  for (let tries = 0; rooms.length < target && tries < target * 30; tries++) {
    const w = 4 + Math.floor(rnd() * 9), h = 4 + Math.floor(rnd() * 9);
    const x = 1 + Math.floor(rnd() * (cols - w - 2)), y = 1 + Math.floor(rnd() * (rows - h - 2));
    if (rooms.some(r => x < r.x + r.w + 2 && x + w + 2 > r.x && y < r.y + r.h + 2 && y + h + 2 > r.y)) continue;
    rooms.push({ x, y, w, h, kind: rnd() < 0.15 ? 4 : rnd() < 0.3 ? 5 : 1 });
  }
  const set = (x, y, v, onlyIf) => {
    if (x < 0 || y < 0 || x >= cols || y >= rows) return;
    if (onlyIf !== undefined && cells[y * cols + x] !== onlyIf) return;
    cells[y * cols + x] = v;
  };
  for (let i = 1; i < rooms.length; i++) { // L corridors to previous room
    const a = rooms[i - 1], b = rooms[i];
    const ax = a.x + (a.w >> 1), ay = a.y + (a.h >> 1), bx = b.x + (b.w >> 1), by = b.y + (b.h >> 1);
    for (let x = Math.min(ax, bx); x <= Math.max(ax, bx); x++) { set(x, ay, 3); set(x, ay + 1, 3); }
    for (let y = Math.min(ay, by); y <= Math.max(ay, by); y++) { set(bx, y, 3); set(bx + 1, y, 3); }
  }
  for (const r of rooms) {
    for (let y = r.y - 1; y <= r.y + r.h; y++) for (let x = r.x - 1; x <= r.x + r.w; x++) {
      const edge = y === r.y - 1 || y === r.y + r.h || x === r.x - 1 || x === r.x + r.w;
      if (edge) set(x, y, 2, 0); else set(x, y, r.kind === 4 && (x === r.x || y === r.y || x === r.x + r.w - 1 || y === r.y + r.h - 1) ? 1 : r.kind);
    }
  }
  return cells;
}

const PAL = {
  0: [38, 34, 32], 1: [150, 132, 108], 2: [92, 88, 84], 3: [128, 112, 92], 4: [46, 92, 128], 5: [120, 126, 136],
};

function render(size, seed) {
  const cols = Math.ceil(size / CELL), rows = cols;
  const cells = makeCells(cols, rows, seed);
  const noise = makeNoise(seed + 1);
  const cellJitter = new Float32Array(cols * rows).map(mulberry32(seed + 2));
  const buf = Buffer.allocUnsafe(size * size * 3);
  const t0 = Date.now();
  for (let y = 0; y < size; y++) {
    const cy = (y / CELL) | 0, iy = y - cy * CELL, ny = (y & (NT - 1)) * NT, ny2 = ((y * 7 >> 3) & (NT - 1)) * NT;
    for (let x = 0; x < size; x++) {
      const cx = (x / CELL) | 0, ix = x - cx * CELL, ci = cy * cols + cx;
      const t = cells[ci];
      let n = noise[ny + (x & (NT - 1))] * 0.7 + noise[ny2 + (((x * 7) >> 3) + 311 & (NT - 1))] * 0.3;
      const p = PAL[t];
      let k;
      if (t === 0) k = 0.55 + n * 0.9;
      else if (t === 4) k = 0.75 + 0.5 * Math.sin((x + y) * 0.02 + n * 6) * 0.25 + n * 0.3;
      else {
        k = 0.8 + n * 0.45 + (cellJitter[ci] - 0.5) * 0.25;
        // flagstone seams inside cells (two stones per cell)
        if (t !== 2 && (ix === 0 || iy === 0 || ix === 50 && (cx & 1) || iy === 50 && (cy & 1))) k *= 0.7;
        if (t === 5 && ((cx % 3 === 1 && cy % 3 === 1) && (ix - 50) ** 2 + (iy - 50) ** 2 < 900)) k = 1.25 + n * 0.2; // pillars
      }
      // dark 2px grid line every square
      if (ix < 2 || iy < 2) k *= 0.6;
      // per-pixel grain so JPEG size and decode cost resemble painted art
      k *= 0.9 + (((Math.imul(x, 73856093) ^ Math.imul(y, 19349663)) >>> 0) % 1000) * 0.0002;
      const o = (y * size + x) * 3;
      buf[o] = Math.min(255, p[0] * k); buf[o + 1] = Math.min(255, p[1] * k); buf[o + 2] = Math.min(255, p[2] * k);
    }
  }
  console.log(`rendered ${size}px in ${Date.now() - t0} ms`);
  return buf;
}

for (const size of sizes) {
  const buf = render(size, 1234);
  const img = () => sharp(buf, { raw: { width: size, height: size, channels: 3 }, limitInputPixels: false });
  let t = Date.now();
  await img().jpeg({ quality: 85, mozjpeg: false }).toFile(join(outDir, `map-${size}.jpg`));
  console.log(`map-${size}.jpg in ${Date.now() - t} ms`);
  t = Date.now();
  const tileRoot = join(outDir, `tiles-${size}`);
  if (existsSync(tileRoot)) rmSync(tileRoot, { recursive: true });
  const tmp = join(outDir, `dz-${size}`);
  await img().jpeg({ quality: 85 }).tile({ size: TILE, overlap: 0, layout: 'dz' }).toFile(tmp + '.dz');
  renameSync(tmp + '_files', tileRoot);
  rmSync(tmp + '.dzi', { force: true });
  const maxLevel = Math.ceil(Math.log2(size));
  const levels = [];
  for (let l = 0; l <= maxLevel; l++) {
    const s = 2 ** (l - maxLevel), w = Math.ceil(size * s), h = Math.ceil(size * s);
    levels.push({ level: l, scale: s, width: w, height: h, cols: Math.ceil(w / TILE), rows: Math.ceil(h / TILE) });
  }
  writeFileSync(join(tileRoot, 'manifest.json'), JSON.stringify({ width: size, height: size, tileSize: TILE, ext: 'jpeg', maxLevel, levels }, null, 1));
  console.log(`tiles-${size} in ${Date.now() - t} ms`);
}

// Patch for the layer test: a 1024² "kitchen" overlay with soft alpha edge.
{
  const S = 1024, buf = Buffer.alloc(S * S * 4);
  for (let y = 0; y < S; y++) for (let x = 0; x < S; x++) {
    const o = (y * S + x) * 4, d = Math.min(x, y, S - 1 - x, S - 1 - y);
    const check = ((x >> 6) + (y >> 6)) & 1;
    buf[o] = check ? 200 : 170; buf[o + 1] = check ? 190 : 120; buf[o + 2] = check ? 150 : 80;
    buf[o + 3] = Math.min(255, d * 4);
  }
  await sharp(buf, { raw: { width: S, height: S, channels: 4 } }).png().toFile(join(outDir, 'patch.png'));
}
console.log('done');
