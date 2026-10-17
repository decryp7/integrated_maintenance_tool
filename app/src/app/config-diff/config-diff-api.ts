import { CompareOptions, DiffExported, DiffExportFormat, DiffFormats, DiffOptions, DiffPage, SnapshotDiff } from '../core/config-diff.types';

/** The engine operations the configuration diff view uses. */
export abstract class ConfigDiffBackend {
  abstract formats(): Promise<DiffFormats>;
  abstract diff(leftPath: string, rightPath: string, options: DiffOptions): Promise<DiffPage>;
  abstract snapshots(leftDir: string, rightDir: string, compare: CompareOptions): Promise<SnapshotDiff>;
  /** Writes the diff; never replaces an existing file unless `overwrite`. */
  abstract exportDiff(leftPath: string, rightPath: string, options: DiffOptions, format: DiffExportFormat, path: string, overwrite?: boolean): Promise<DiffExported>;
}
