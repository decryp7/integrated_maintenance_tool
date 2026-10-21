import { formatBytes } from '../shared/format/units';
import { Injectable } from '@angular/core';
import { iconNameFromFile } from './marker-rules';
import { PendingIcon } from './markers.types';

/** The engine's limit for icon sources (crates/imt-core/src/icons.rs MAX_ICON_BYTES). */
export const MAX_ICON_BYTES = 16 * 1024;
const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47];

export type IconImport = { ok: true; icon: PendingIcon } | { ok: false; error: string };

function toBase64(bytes: Uint8Array): string {
  let bin = '';
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(bin);
}

/** Reads a chosen SVG or PNG file into a pending icon; the engine still validates and sanitises on save. */
export async function readIconFile(file: File, takenNames: ReadonlySet<string>): Promise<IconImport> {
  if (file.size > MAX_ICON_BYTES) return { ok: false, error: $localize`:@@markers.iconImport.tooBig:Icons are limited to ${formatBytes(MAX_ICON_BYTES)}:limit:; this file is ${formatBytes(file.size)}:size:.` };
  const name = iconNameFromFile(file.name, takenNames);
  try {
    if (/\.png$/i.test(file.name) || file.type === 'image/png') {
      const bytes = new Uint8Array(await file.arrayBuffer());
      if (!PNG_SIGNATURE.every((b, i) => bytes[i] === b)) return { ok: false, error: $localize`:@@markers.iconImport.notPng:That file is not a PNG image.` };
      const png = toBase64(bytes);
      return { ok: true, icon: { name, png, pngDataUrl: `data:image/png;base64,${png}` } };
    }
    if (/\.svg$/i.test(file.name) || file.type === 'image/svg+xml') {
      const svg = await file.text();
      if (!/<svg[\s>]/i.test(svg)) return { ok: false, error: $localize`:@@markers.iconImport.notSvg:That file is not an SVG image.` };
      return { ok: true, icon: { name, svg } };
    }
  } catch {
    return { ok: false, error: $localize`:@@markers.iconImport.unreadable:The file could not be read.` };
  }
  return { ok: false, error: $localize`:@@markers.iconImport.wrongType:Choose an .svg or .png file.` };
}

/**
 * Asks the user for an icon file. Uses the webview's own file chooser: it hands the page the file's
 * bytes without any filesystem permission (the Tauri dialog plugin only returns paths, and the app
 * deliberately has no fs capability). Replaceable in tests.
 */
@Injectable({ providedIn: 'root' })
export class IconFilePicker {
  pick(): Promise<File | null> {
    return new Promise((resolve) => {
      const input = document.createElement('input');
      input.type = 'file';
      input.accept = '.svg,.png,image/svg+xml,image/png';
      input.addEventListener('change', () => resolve(input.files?.[0] ?? null), { once: true });
      input.addEventListener('cancel', () => resolve(null), { once: true });
      input.click();
    });
  }
}
