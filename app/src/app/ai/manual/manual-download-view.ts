/**
 * Pure display rules of "Download manually": which failures offer it and the headline per failure kind.
 * The engine classifies the failure (the `code`) and writes the detailed message; this only names it.
 */
import { AiErrorCode } from '../ai.types';

/** Download failures the user can work around by downloading the file in the browser. */
const MANUAL_CODES: readonly AiErrorCode[] = [
  'tls',
  'proxyAuth',
  'proxy',
  'offline',
  'timeout',
  'httpStatus',
  'network',
];

export function offersManualDownload(code: AiErrorCode | null | undefined): boolean {
  return !!code && MANUAL_CODES.includes(code);
}

/** A short headline for the failure kind; empty for kinds without one. */
export function failureHeadline(code: AiErrorCode | null | undefined): string {
  switch (code) {
    case 'tls':
      return $localize`:@@ai.manual.kind.tls:Certificate problem: a proxy or security tool on your network intercepts HTTPS`;
    case 'proxyAuth':
      return $localize`:@@ai.manual.kind.proxyAuth:The proxy asks for a sign-in`;
    case 'proxy':
      return $localize`:@@ai.manual.kind.proxy:The download proxy cannot be used`;
    case 'offline':
      return $localize`:@@ai.manual.kind.offline:The server cannot be reached`;
    case 'timeout':
      return $localize`:@@ai.manual.kind.timeout:The server did not answer in time`;
    case 'httpStatus':
      return $localize`:@@ai.manual.kind.httpStatus:The server refused the download`;
    case 'network':
      return $localize`:@@ai.manual.kind.network:The connection failed`;
    default:
      return '';
  }
}

/** The host part of an https address, for "from huggingface.co". */
export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return '';
  }
}
