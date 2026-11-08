// Minimal PNG reader for the smoke checks: 8-bit RGB / RGBA, non-interlaced (what the export
// writes and what fixtures/gen generates). Node's zlib only; no dependency.
import { readFileSync } from 'node:fs';
import { inflateSync } from 'node:zlib';

const SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

/** The chunk types in file order, the IHDR fields and the decoded pixels (RGB8, row-major). */
export function readPng(path) {
  const buf = readFileSync(path);
  if (!buf.subarray(0, 8).equals(SIGNATURE)) throw new Error(`${path}: not a PNG`);
  const chunks = [];
  const idat = [];
  let ihdr = null;
  for (let at = 8; at < buf.length;) {
    const len = buf.readUInt32BE(at);
    const type = buf.toString('latin1', at + 4, at + 8);
    const data = buf.subarray(at + 8, at + 8 + len);
    chunks.push(type);
    if (type === 'IHDR')
      ihdr = {
        width: data.readUInt32BE(0),
        height: data.readUInt32BE(4),
        depth: data[8],
        color: data[9],
        interlace: data[12],
      };
    if (type === 'IDAT') idat.push(data);
    at += 12 + len;
  }
  if (!ihdr) throw new Error(`${path}: no IHDR`);
  const { width, height, depth, color, interlace } = ihdr;
  const channels = { 2: 3, 6: 4 }[color];
  if (depth !== 8 || !channels || interlace !== 0)
    throw new Error(
      `${path}: unsupported PNG (depth ${depth}, color ${color}, interlace ${interlace})`,
    );
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  if (raw.length !== height * (stride + 1))
    throw new Error(
      `${path}: ${raw.length} bytes of image data, expected ${height * (stride + 1)}`,
    );
  const px = Buffer.alloc(height * stride);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const src = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const row = px.subarray(y * stride, (y + 1) * stride);
    const up = y > 0 ? px.subarray((y - 1) * stride, y * stride) : null;
    for (let i = 0; i < stride; i++) {
      const a = i >= channels ? row[i - channels] : 0;
      const b = up ? up[i] : 0;
      const c = up && i >= channels ? up[i - channels] : 0;
      let v = src[i];
      if (filter === 1) v += a;
      else if (filter === 2) v += b;
      else if (filter === 3) v += (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a);
        const pb = Math.abs(p - b);
        const pc = Math.abs(p - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      } else if (filter !== 0) throw new Error(`${path}: bad filter ${filter} on row ${y}`);
      row[i] = v & 0xff;
    }
  }
  let rgb = px;
  if (channels === 4) {
    rgb = Buffer.alloc(width * height * 3);
    for (let i = 0, j = 0; i < px.length; i += 4, j += 3) px.copy(rgb, j, i, i + 3);
  }
  return { chunks, width, height, rgb };
}
