import { MockDataset } from './mock-data';

/** A 200+ character Windows path without a space in it (the worst case for wrapping). */
export const LONG_WINDOWS_PATH =
  'C:\\ProgramData\\VeryLongVendorNameIncorporated\\ExtremelyDescriptiveProductSuiteName\\InstalledComponents\\CompanyA.Graphic.Builder.Service\\Diagnostics\\Output\\CompanyA.Graphic.Builder.Service.Diagnostics.2026-10-23.log';
export const LONG_POSIX_PATH =
  '/var/lib/very-long-vendor-name/extremely-descriptive-product-suite-name/installed-components/company-a-graphic-builder-service/diagnostics/output/company-a.graphic-builder.service.diagnostics.2026-10-23.log';
export const LONG_FILE_NAME = 'CompanyA.Graphic.Builder.Service.Diagnostics.With.A.Very.Long.Single.Token.Name.2026-10-23T09-41-24.Rotated.0001.log';

const fileName = (path: string): string => path.split(/[\\/]/).pop() ?? path;

/** The demo incident plus files with very long paths and names (UI tests: `?mockLongNames=1`). */
export function withLongNames(demo: MockDataset): MockDataset {
  const sources = demo.sources.map((s, i) => (i === 0 ? { ...s, path: LONG_WINDOWS_PATH, name: fileName(LONG_WINDOWS_PATH) } : i === 1 ? { ...s, path: LONG_POSIX_PATH, name: fileName(LONG_POSIX_PATH) } : s));
  const unrecognized = [
    ...demo.unrecognized,
    { path: LONG_WINDOWS_PATH.replace('.log', '.dmp'), name: fileName(LONG_WINDOWS_PATH).replace('.log', '.dmp'), reason: `No importer reads ${LONG_WINDOWS_PATH.replace('.log', '.dmp')}` },
    { path: LONG_POSIX_PATH.replace('.log', '.bin'), name: fileName(LONG_POSIX_PATH).replace('.log', '.bin'), reason: 'Unrecognised binary format' },
    { path: `/data/${LONG_FILE_NAME}.bak`, name: `${LONG_FILE_NAME}.bak`, reason: 'No importer for this file' },
  ];
  return { ...demo, sources, unrecognized };
}
