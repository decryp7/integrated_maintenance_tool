import { EncodingInfo } from '../../core/api.types';

/** Below this confidence a guessed encoding is flagged as doubtful. */
export const LOW_CONFIDENCE = 70;

/** The "Automatic (detect)" choice (the engine guesses the encoding again). */
export const AUTO_ENCODING = '';

/** Whether the engine only guessed the encoding and is not sure. */
export function isLowConfidence(info: EncodingInfo | null | undefined): boolean {
  return !!info && info.origin === 'detected' && info.confidence < LOW_CONFIDENCE;
}

/**
 * The encoding as one line: "Shift_JIS · 84% (detected)", "UTF-16LE (byte-order mark)", "windows-1252 (chosen)" or
 * "GBK (profile)". Only a guess shows its confidence.
 */
export function encodingText(info: EncodingInfo): string {
  switch (info.origin) {
    case 'detected': {
      const { label, confidence } = info;
      return $localize`:@@sources.encoding.text.detected:${label}:label: · ${confidence}:confidence:% (detected)`;
    }
    case 'bom': {
      const { label } = info;
      return $localize`:@@sources.encoding.text.bom:${label}:label: (byte-order mark)`;
    }
    case 'profile': {
      const { label } = info;
      return $localize`:@@sources.encoding.text.profile:${label}:label: (profile)`;
    }
    case 'manual': {
      const { label } = info;
      return $localize`:@@sources.encoding.text.manual:${label}:label: (chosen)`;
    }
  }
}

/** The tooltip of the encoding: the line, and for a doubtful guess how to fix it. */
export function encodingTitle(info: EncodingInfo): string {
  const text = encodingText(info);
  if (!isLowConfidence(info)) return text;
  return `${text}\n${$localize`:@@sources.encoding.lowConfidence:Guessed; choose another encoding if the text looks wrong.`}`;
}

/** The short badge text: the label, with the confidence when it is a guess that is not certain. */
export function encodingBadge(info: EncodingInfo): string {
  return info.origin === 'detected' && info.confidence < 100 ? `${info.label} ${info.confidence}%` : info.label;
}

/** Whether the Explorer row shows a badge: everything but a confidently detected (or BOM-marked) UTF-8, the unremarkable case. */
export function showsEncodingBadge(info: EncodingInfo | null | undefined): info is EncodingInfo {
  if (!info) return false;
  return !(info.label === 'UTF-8' && info.origin !== 'manual' && !isLowConfidence(info));
}

/** What the dialog selects at first: the encoding the user chose before, else "Automatic". */
export function initialEncodingChoice(info: EncodingInfo | null | undefined): string {
  return info?.origin === 'manual' ? info.label : AUTO_ENCODING;
}
