// Guards the app icons: the artwork must fill the canvas. (v1.0.0 shipped placeholders with the drawing in the top-left
// corner only and the rest of the canvas plain white, so the installed app showed a blank icon.) Artwork pixels are
// those that are visible (alpha > 0) and not near-white, so both a transparent and a white empty canvas count as empty.
//   node scripts/ci/check-icons.mjs [icons-dir]     default: app/src-tauri/icons
import { readFileSync } from 'node:fs';
import { inflateSync } from 'node:zlib';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

export const ICONS = ['icon.png', '128x128@2x.png', '128x128.png', '32x32.png'];
const MIN_COVERAGE = 0.7;
const WHITE = 245;
const MAX_OFFCENTRE = 0.1;
const SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

/** Decodes an 8-bit non-interlaced RGBA PNG into { width, height, alpha(x, y) }. Throws on anything else. */
export function decodePng(buf) {
  if (buf.length < 8 || !buf.subarray(0, 8).equals(SIGNATURE)) throw new Error('not a PNG file');
  let pos = 8;
  let ihdr = null;
  const idat = [];
  while (pos + 8 <= buf.length) {
    const len = buf.readUInt32BE(pos);
    const type = buf.toString('latin1', pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === 'IHDR') ihdr = data;
    else if (type === 'IDAT') idat.push(data);
    else if (type === 'IEND') break;
    pos += 12 + len;
  }
  if (!ihdr) throw new Error('PNG has no IHDR chunk');
  const width = ihdr.readUInt32BE(0);
  const height = ihdr.readUInt32BE(4);
  const [depth, colour, , , interlace] = [ihdr[8], ihdr[9], ihdr[10], ihdr[11], ihdr[12]];
  if (depth !== 8 || colour !== 6 || interlace !== 0) {
    throw new Error(`unsupported PNG (bit depth ${depth}, colour type ${colour}, interlace ${interlace}); expected 8-bit RGBA, not interlaced`);
  }
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * 4;
  if (raw.length !== (stride + 1) * height) throw new Error('PNG pixel data has the wrong size');
  const out = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const src = y * (stride + 1) + 1;
    for (let i = 0; i < stride; i++) {
      const a = i >= 4 ? out[y * stride + i - 4] : 0;
      const b = y > 0 ? out[(y - 1) * stride + i] : 0;
      const c = i >= 4 && y > 0 ? out[(y - 1) * stride + i - 4] : 0;
      let v = raw[src + i];
      if (filter === 1) v += a;
      else if (filter === 2) v += b;
      else if (filter === 3) v += (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      } else if (filter !== 0) throw new Error(`bad PNG filter ${filter}`);
      out[y * stride + i] = v & 255;
    }
  }
  return {
    width,
    height,
    alpha: (x, y) => out[y * stride + x * 4 + 3],
    /** Visible and not near-white. */
    artwork: (x, y) => {
      const o = y * stride + x * 4;
      return out[o + 3] > 0 && (out[o] < WHITE || out[o + 1] < WHITE || out[o + 2] < WHITE);
    },
  };
}

/** Bounding box of the artwork pixels, or null when the image has none. */
export function opaqueBounds({ width, height, artwork }) {
  let x0 = width, y0 = height, x1 = -1, y1 = -1;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      if (artwork(x, y)) {
        if (x < x0) x0 = x;
        if (x > x1) x1 = x;
        if (y < y0) y0 = y;
        if (y > y1) y1 = y;
      }
    }
  }
  return x1 < 0 ? null : { x0, y0, x1, y1 };
}

/** Problems with one icon (empty list when it is fine). */
export function problems(name, png) {
  const b = opaqueBounds(png);
  if (!b) return [`${name}: empty (fully transparent or white)`];
  const w = b.x1 - b.x0 + 1, h = b.y1 - b.y0 + 1;
  const out = [];
  if (w / png.width < MIN_COVERAGE) out.push(`${name}: artwork covers only ${Math.round((100 * w) / png.width)}% of the width`);
  if (h / png.height < MIN_COVERAGE) out.push(`${name}: artwork covers only ${Math.round((100 * h) / png.height)}% of the height`);
  const dx = (b.x0 + b.x1 + 1) / 2 / png.width - 0.5;
  const dy = (b.y0 + b.y1 + 1) / 2 / png.height - 0.5;
  if (Math.abs(dx) > MAX_OFFCENTRE || Math.abs(dy) > MAX_OFFCENTRE) out.push(`${name}: artwork is not centred (offset ${dx.toFixed(2)}, ${dy.toFixed(2)})`);
  return out;
}

export function checkDir(dir) {
  return ICONS.flatMap((name) => {
    try {
      return problems(name, decodePng(readFileSync(join(dir, name))));
    } catch (e) {
      return [`${name}: ${e.message}`];
    }
  });
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const dir = process.argv[2] ?? 'app/src-tauri/icons';
  const found = checkDir(dir);
  if (found.length) {
    for (const p of found) console.error(`error: ${p}`);
    process.exit(1);
  }
  console.log(`icons ok (${ICONS.join(', ')} in ${dir})`);
}
