#!/usr/bin/env node
// Generated fixture images (KQ-22): procedural battlemaps, no real art. Deterministic: the same
// arguments always give byte-identical files. Node built-ins only (zlib PNG encoder).
//
//   corepack pnpm fixtures                       → the default set into fixtures/generated/
//   corepack pnpm fixtures -- --size 4200x5600   → one extra map
//   corepack pnpm fixtures -- --size 4200x5600 --cell 140  → with 140 px cells (EXP walkthrough:
//                                                    30×40 cells)
//   corepack pnpm fixtures -- --committed        → rewrite the small committed fixtures
//
// Committed fixtures (< 1 MB each, AGENTS §9) live in fixtures/images/; everything else goes to
// fixtures/generated/ (gitignored).
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { crc32, deflateSync } from 'node:zlib';

const root = dirname(dirname(fileURLToPath(import.meta.url)));

function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const td = Buffer.concat([Buffer.from(type, 'latin1'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(td) >>> 0);
  return Buffer.concat([len, td, crc]);
}

/** Encodes RGB8 pixels as a PNG (filter 0, zlib level 6: deterministic). */
export function encodePng(width, height, rgb) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr.set([8, 2, 0, 0, 0], 8);
  const raw = Buffer.alloc((width * 3 + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (width * 3 + 1)] = 0;
    rgb.copy(raw, y * (width * 3 + 1) + 1, y * width * 3, (y + 1) * width * 3);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 6 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/**
 * A procedural dungeon battlemap: stone floor noise, rooms, walls and a grid every `cell` px.
 * Corner pixels encode orientation (top-left red, bottom-right blue) for view tests.
 */
export function gridMap(width, height, { cell = 70, seed = 7 } = {}) {
  const rnd = mulberry32(seed);
  const cols = Math.ceil(width / cell);
  const rows = Math.ceil(height / cell);
  // 0 rock, 1 floor
  const cells = new Uint8Array(cols * rows);
  const rooms = Math.max(3, Math.floor((cols * rows) / 60));
  for (let r = 0; r < rooms; r++) {
    const w = 3 + Math.floor(rnd() * 6);
    const h = 3 + Math.floor(rnd() * 6);
    const x0 = Math.floor(rnd() * Math.max(1, cols - w));
    const y0 = Math.floor(rnd() * Math.max(1, rows - h));
    for (let y = y0; y < Math.min(rows, y0 + h); y++)
      for (let x = x0; x < Math.min(cols, x0 + w); x++) cells[y * cols + x] = 1;
  }
  const lattice = new Float32Array(257 * 257).map(() => rnd());
  const rgb = Buffer.alloc(width * height * 3);
  for (let y = 0; y < height; y++) {
    const cy = Math.floor(y / cell);
    for (let x = 0; x < width; x++) {
      const cx = Math.floor(x / cell);
      const floor = cells[cy * cols + cx] === 1;
      const n = lattice[((y >> 3) % 257) * 257 + ((x >> 3) % 257)];
      const onGrid = x % cell === 0 || y % cell === 0;
      let r, g, b;
      if (floor) {
        const v = 120 + n * 40;
        [r, g, b] = [v, v * 0.95, v * 0.85];
        if (onGrid) [r, g, b] = [r * 0.6, g * 0.6, b * 0.6];
      } else {
        const v = 40 + n * 25;
        [r, g, b] = [v, v * 1.05, v * 1.1];
      }
      const i = (y * width + x) * 3;
      rgb[i] = r;
      rgb[i + 1] = g;
      rgb[i + 2] = b;
    }
  }
  const mark = (x0, y0, color) => {
    for (let y = y0; y < y0 + 8; y++)
      for (let x = x0; x < x0 + 8; x++) rgb.set(color, (y * width + x) * 3);
  };
  mark(0, 0, [255, 0, 0]);
  mark(width - 8, height - 8, [0, 0, 255]);
  return encodePng(width, height, rgb);
}

function write(path, bytes) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, bytes);
  console.log(`${path.slice(root.length + 1)}  ${(bytes.length / 1024).toFixed(0)} KiB`);
}

function parseSize(s) {
  const m = /^(\d+)x(\d+)$/.exec(s ?? '');
  if (!m) throw new Error(`--size needs WxH, got ${s}`);
  return [Number(m[1]), Number(m[2])];
}

const args = process.argv.slice(2).filter((a) => a !== '--');
if (args.includes('--committed')) {
  // Small fixtures used by e2e and docs. Keep each < 1 MB.
  write(join(root, 'images', 'grid-map-512x640.png'), gridMap(512, 640, { cell: 64 }));
  // Export goldens base (export spec §13): 1024×1280, 8×10 cells at 128 px.
  write(join(root, 'images', 'grid-map-1024x1280.png'), gridMap(1024, 1280, { cell: 128 }));
} else if (args.includes('--size')) {
  const [w, h] = parseSize(args[args.indexOf('--size') + 1]);
  const cell = args.includes('--cell') ? Number(args[args.indexOf('--cell') + 1]) : undefined;
  if (cell !== undefined && !(Number.isInteger(cell) && cell >= 8))
    throw new Error(`--cell needs a whole number of px (>= 8), got ${cell}`);
  write(join(root, 'generated', `grid-map-${w}x${h}.png`), gridMap(w, h, { cell }));
} else {
  // Typical map (#30): 30×40 cells at 70 px.
  write(join(root, 'generated', 'grid-map-2100x2800.png'), gridMap(2100, 2800));
}
