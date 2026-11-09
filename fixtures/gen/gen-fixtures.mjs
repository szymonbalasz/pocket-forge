#!/usr/bin/env node
// Generated fixture images (KQ-22): procedural battlemaps, no real art. Deterministic: the same
// arguments always give byte-identical files. Node built-ins only (zlib PNG encoder).
//
//   corepack pnpm fixtures                       → the default set into fixtures/generated/
//   corepack pnpm fixtures -- --size 4200x5600   → one extra map
//   corepack pnpm fixtures -- --size 4200x5600 --cell 140  → with 140 px cells (EXP walkthrough:
//                                                    30×40 cells)
//   corepack pnpm fixtures -- --committed        → rewrite the small committed fixtures
//                                                    (incl. GEN's sketch, fixtures/gen/)
//
// Committed fixtures (< 1 MB each, AGENTS §9) live in fixtures/images/; everything else goes to
// fixtures/generated/ (gitignored).
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { crc32, deflateSync } from "node:zlib";

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
  const td = Buffer.concat([Buffer.from(type, "latin1"), data]);
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
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 6 })),
    chunk("IEND", Buffer.alloc(0)),
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

/**
 * GEN §13's structural sketch: a line-art room layout, black walls on white, no grid or text.
 * Rooms are rectangles on a coarse lattice joined by door gaps; deterministic per seed.
 */
export function roomLayout(width = 768, height = 1024, { seed = 59 } = {}) {
  const rand = mulberry32(seed);
  const rgb = Buffer.alloc(width * height * 3, 255);
  const wall = 6;
  const ink = (x0, y0, x1, y1) => {
    for (let y = Math.max(0, y0); y < Math.min(height, y1); y++)
      for (let x = Math.max(0, x0); x < Math.min(width, x1); x++)
        rgb.fill(0, (y * width + x) * 3, (y * width + x) * 3 + 3);
  };
  const clear = (x0, y0, x1, y1) => {
    for (let y = Math.max(0, y0); y < Math.min(height, y1); y++)
      for (let x = Math.max(0, x0); x < Math.min(width, x1); x++)
        rgb.fill(255, (y * width + x) * 3, (y * width + x) * 3 + 3);
  };
  const margin = 48;
  // A 3 × 4 lattice of rooms with uneven sizes.
  const xs = [margin];
  for (let i = 1; i < 3; i++)
    xs.push(
      margin + Math.round(((width - 2 * margin) * i) / 3 + (rand() - 0.5) * 80),
    );
  xs.push(width - margin);
  const ys = [margin];
  for (let i = 1; i < 4; i++)
    ys.push(
      margin +
        Math.round(((height - 2 * margin) * i) / 4 + (rand() - 0.5) * 80),
    );
  ys.push(height - margin);
  for (const x of xs)
    ink(
      x - wall / 2,
      margin - wall / 2,
      x + wall / 2,
      height - margin + wall / 2,
    );
  for (const y of ys)
    ink(
      margin - wall / 2,
      y - wall / 2,
      width - margin + wall / 2,
      y + wall / 2,
    );
  // A door in every inner wall segment.
  const door = 40;
  for (let i = 1; i < xs.length - 1; i++)
    for (let j = 0; j < ys.length - 1; j++) {
      const mid = Math.round(
        ys[j] + (ys[j + 1] - ys[j]) * (0.3 + rand() * 0.4),
      );
      clear(xs[i] - wall, mid - door / 2, xs[i] + wall, mid + door / 2);
    }
  for (let j = 1; j < ys.length - 1; j++)
    for (let i = 0; i < xs.length - 1; i++) {
      const mid = Math.round(
        xs[i] + (xs[i + 1] - xs[i]) * (0.3 + rand() * 0.4),
      );
      clear(mid - door / 2, ys[j] - wall, mid + door / 2, ys[j] + wall);
    }
  // The entrance at the bottom.
  clear(
    width / 2 - door,
    height - margin - wall,
    width / 2 + door,
    height - margin + wall,
  );
  return encodePng(width, height, rgb);
}

function write(path, bytes) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, bytes);
  console.log(
    `${path.slice(root.length + 1)}  ${(bytes.length / 1024).toFixed(0)} KiB`,
  );
}

function parseSize(s) {
  const m = /^(\d+)x(\d+)$/.exec(s ?? "");
  if (!m) throw new Error(`--size needs WxH, got ${s}`);
  return [Number(m[1]), Number(m[2])];
}

const args = process.argv.slice(2).filter((a) => a !== "--");
if (args.includes("--committed")) {
  // Small fixtures used by e2e and docs. Keep each < 1 MB.
  write(
    join(root, "images", "grid-map-512x640.png"),
    gridMap(512, 640, { cell: 64 }),
  );
  // Export goldens base (export spec §13): 1024×1280, 8×10 cells at 128 px.
  write(
    join(root, "images", "grid-map-1024x1280.png"),
    gridMap(1024, 1280, { cell: 128 }),
  );
  // GEN §13: the structural sketch, a 768×1024 line-art room layout.
  write(join(root, "gen", "room-layout-768x1024.png"), roomLayout(768, 1024));
} else if (args.includes("--size")) {
  const [w, h] = parseSize(args[args.indexOf("--size") + 1]);
  const cell = args.includes("--cell")
    ? Number(args[args.indexOf("--cell") + 1])
    : undefined;
  if (cell !== undefined && !(Number.isInteger(cell) && cell >= 8))
    throw new Error(`--cell needs a whole number of px (>= 8), got ${cell}`);
  write(
    join(root, "generated", `grid-map-${w}x${h}.png`),
    gridMap(w, h, { cell }),
  );
} else {
  // Typical map (#30): 30×40 cells at 70 px.
  write(join(root, "generated", "grid-map-2100x2800.png"), gridMap(2100, 2800));
}
