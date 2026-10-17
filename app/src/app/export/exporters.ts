import { ExporterInfo } from '../core/api.types';

/** The exporter whose extension matches the file name; falls back to the first exporter. */
export function exporterForPath(path: string, exporters: readonly ExporterInfo[]): ExporterInfo | null {
  const ext = /\.([^./\\]+)$/.exec(path)?.[1]?.toLowerCase();
  return exporters.find((e) => e.extension.toLowerCase() === ext) ?? exporters[0] ?? null;
}
