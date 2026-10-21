/**
 * Pure helpers for handing images to the engine (ADR 0025). The webview never decodes the user's image: pasted and
 * dropped files are read as bytes and sent base64, picked and Explorer files as paths. Rust decodes, bounds and
 * re-encodes them; the limits here only fail early for convenience (the engine re-checks).
 */

/** The image formats the engine reads, by extension (what the file pickers offer and the Explorer recognises). */
export const IMAGE_EXTENSIONS: readonly string[] = ['png', 'jpg', 'jpeg', 'bmp', 'gif', 'webp'];

/** At most this many images go with one question (the worker re-checks). */
export const MAX_IMAGES_PER_QUESTION = 4;

/** The engine's size limit of one image file. */
export const MAX_IMAGE_BYTES = 32 * 1024 * 1024;

const MIME = new Set(['image/png', 'image/jpeg', 'image/bmp', 'image/gif', 'image/webp']);

/** True when the path names an image the engine reads (by extension; the engine checks the signature). */
export function isImagePath(path: string): boolean {
  const ext = /\.([a-z0-9]+)$/i.exec(path)?.[1]?.toLowerCase();
  return !!ext && IMAGE_EXTENSIONS.includes(ext);
}

/** True when a pasted or dropped file looks like an image the engine reads (its type, else its name). */
export function isImageFile(file: { type: string; name: string }): boolean {
  return file.type ? MIME.has(file.type.toLowerCase()) : isImagePath(file.name);
}

/** The image files of a paste or drop (`clipboardData` / `dataTransfer`), in order. */
export function imageFilesOf(data: Pick<DataTransfer, 'files' | 'items'> | null | undefined): File[] {
  if (!data) return [];
  const files = data.files?.length ? [...data.files] : [...(data.items ?? [])].filter((i) => i.kind === 'file').map((i) => i.getAsFile()).filter((f): f is File => !!f);
  return files.filter(isImageFile);
}

/** True when a drag carries files (so the composer accepts the drop). */
export function dragHasFiles(data: Pick<DataTransfer, 'types'> | null | undefined): boolean {
  return !!data && [...(data.types ?? [])].includes('Files');
}

/** Bytes a chunk of `String.fromCharCode` turns into text at once (well under engines' argument limits). */
const CHUNK = 0x8000;

/** Base64 of raw bytes, built in chunks (no string concatenation per byte, no decoding). */
export function bytesToBase64(bytes: Uint8Array): string {
  const parts: string[] = [];
  for (let i = 0; i < bytes.length; i += CHUNK) parts.push(String.fromCharCode(...bytes.subarray(i, i + CHUNK)));
  return btoa(parts.join(''));
}

/** The image file picker's filter. */
export function imageFilter(): { name: string; extensions: string[] }[] {
  return [{ name: $localize`:@@ai.vision.filter:Images`, extensions: [...IMAGE_EXTENSIONS] }];
}
