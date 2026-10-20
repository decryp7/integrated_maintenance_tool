// Regenerates IMT's app icons from the SVG masters in src-tauri/icons-src (see docs/development.md, "App icon").
// Run from app/: node scripts/app-icon.mjs
//
// 1. `tauri icon` renders every bundle icon (ico, icns, PNGs, Store logos) from imt-icon.svg.
// 2. The small sizes (16/24/32 px in the .ico, 32x32.png, Square30x30Logo.png) are re-rendered from the
//    pixel-snapped imt-icon-small.svg, which stays legible where the master blurs.
// 3. tray-template.png (macOS menu bar) is rendered from imt-tray-template.svg.
// 4. public/favicon.ico is a copy of the patched icon.ico.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Sizes (px) taken from the small-size variant. */
export const SMALL_SIZES = [16, 24, 30, 32];

/**
 * Parses an .ico file into its entries ({ size, data }); `size` is the width (0 in the directory means 256).
 * Throws on anything that is not a well-formed icon directory.
 */
export function parseIco(buf) {
  if (buf.length < 6 || buf.readUInt16LE(0) !== 0 || buf.readUInt16LE(2) !== 1) throw new Error('not an .ico file');
  const count = buf.readUInt16LE(4);
  if (buf.length < 6 + count * 16) throw new Error('truncated .ico directory');
  const entries = [];
  for (let i = 0; i < count; i++) {
    const at = 6 + i * 16;
    const len = buf.readUInt32LE(at + 8);
    const offset = buf.readUInt32LE(at + 12);
    if (offset + len > buf.length) throw new Error(`.ico entry ${i} lies outside the file`);
    entries.push({ size: buf[at] || 256, data: buf.subarray(offset, offset + len) });
  }
  return entries;
}

/** Serialises PNG-compressed entries ({ size, data }) into an .ico file, in the given order. */
export function writeIco(entries) {
  const head = Buffer.alloc(6 + entries.length * 16);
  head.writeUInt16LE(1, 2);
  head.writeUInt16LE(entries.length, 4);
  let offset = head.length;
  entries.forEach(({ size, data }, i) => {
    const at = 6 + i * 16;
    head[at] = size >= 256 ? 0 : size;
    head[at + 1] = size >= 256 ? 0 : size;
    head.writeUInt16LE(1, at + 4); // colour planes
    head.writeUInt16LE(32, at + 6); // bits per pixel
    head.writeUInt32LE(data.length, at + 8);
    head.writeUInt32LE(offset, at + 12);
    offset += data.length;
  });
  return Buffer.concat([head, ...entries.map((e) => e.data)]);
}

/** Replaces the entries whose size has a PNG in `replacements` (size → PNG bytes); the others are kept. */
export function replaceIcoEntries(ico, replacements) {
  return writeIco(parseIco(ico).map((e) => (replacements.has(e.size) ? { size: e.size, data: replacements.get(e.size) } : e)));
}

function tauriIcon(appDir, args) {
  const cli = join(appDir, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
  execFileSync(process.execPath, [cli, 'icon', ...args], { cwd: appDir, stdio: 'inherit' });
}

function main() {
  const appDir = fileURLToPath(new URL('..', import.meta.url));
  const src = join(appDir, 'src-tauri', 'icons-src');
  const icons = join(appDir, 'src-tauri', 'icons');
  tauriIcon(appDir, [join(src, 'imt-icon.svg')]);
  // IMT ships no mobile build: drop the Android/iOS sets `tauri icon` always writes.
  rmSync(join(icons, 'android'), { recursive: true, force: true });
  rmSync(join(icons, 'ios'), { recursive: true, force: true });

  const tmp = mkdtempSync(join(tmpdir(), 'imt-icon-'));
  try {
    tauriIcon(appDir, [join(src, 'imt-icon-small.svg'), '-o', tmp, ...SMALL_SIZES.flatMap((s) => ['-p', String(s)])]);
    const small = new Map(SMALL_SIZES.map((s) => [s, readFileSync(join(tmp, `${s}x${s}.png`))]));
    const ico = join(icons, 'icon.ico');
    writeFileSync(ico, replaceIcoEntries(readFileSync(ico), small));
    writeFileSync(join(icons, '32x32.png'), small.get(32));
    writeFileSync(join(icons, 'Square30x30Logo.png'), small.get(30));
    copyFileSync(ico, join(appDir, 'public', 'favicon.ico'));

    tauriIcon(appDir, [join(src, 'imt-tray-template.svg'), '-o', tmp, '-p', '44']);
    copyFileSync(join(tmp, '44x44.png'), join(icons, 'tray-template.png'));
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
