import { DiagnosticsReportState, DiagnosticsReportView } from '../diagnostics-report.types';

/** A 1x1 PNG, the "screenshot" of the sample report's snapshot. */
export const SAMPLE_REPORT_IMAGE =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==';

/** Matches the path or name of an opened Debrief diagnostics zip. */
export const DIAGNOSTICS_ZIP = /debrief-diagnostics.*\.zip/i;

/** A plausible report for browser development and UI tests. The Rust engine reads the real ones. */
export function sampleReport(name: string): DiagnosticsReportView {
  return {
    name,
    manifest: {
      schemaVersion: 1,
      debriefVersion: '0.9.0',
      commit: 'ac7f592',
      createdAt: '2026-10-14T09:31:12Z',
      files: [
        { name: 'system-info.json', kind: 'system-info', bytes: 1536 },
        { name: 'processes.json', kind: 'processes', bytes: 700 },
        { name: 'logs/children/ai-worker.log', kind: 'child-log', bytes: 4096 },
        { name: 'logs/children/llama-server.log', kind: 'child-log', bytes: 8192 },
      ],
    },
    systemInfo: {
      appVersion: '0.9.0',
      commit: 'ac7f592',
      os: 'windows',
      osVersion: '11 (26100)',
      arch: 'x86_64',
      cpuCores: 16,
      totalMemoryMb: 32768,
      availableMemoryMb: 12044,
      safeMode: false,
      plugins: [
        { id: 'net.decryptology.web', version: '1.2.0', status: 'quarantined' },
        { id: 'net.decryptology.syslog', version: '1.0.3', status: 'active' },
      ],
    },
    processes: [
      { name: 'ai-worker', kind: 'ai-worker', version: '0.9.0', pid: 4120, state: 'running', starts: 1, restarts: 0, lastExit: null, lastExitAt: null, quarantined: false, quarantineReason: null },
      {
        name: 'llama-server',
        kind: 'llama-server',
        version: 'b4567',
        pid: 5312,
        state: 'crashed',
        starts: 2,
        restarts: 1,
        lastExit: 'exit code 3',
        lastExitAt: '2026-10-14T09:29:41Z',
        quarantined: false,
        quarantineReason: null,
      },
      {
        name: 'plugin-net.decryptology.web',
        kind: 'plugin',
        version: '1.2.0',
        pid: null,
        state: 'quarantined',
        starts: 3,
        restarts: 2,
        lastExit: 'signal 11',
        lastExitAt: '2026-10-14T09:28:10Z',
        quarantined: true,
        quarantineReason: 'failed 3 times in a row',
      },
    ],
    steps: ['1. Opened <dir#1a2b3c4d>/samples (3 files) from the menu', "2. Clicked 'Ask the assistant' from the toolbar", '3. The AI engine stopped with exit code 3'],
    breadcrumbs: [
      { time: '2026-10-14T09:27:55Z', operation: 'file.open', trigger: 'menu', window: 'main', outcome: 'ok', params: { Files: '3' } },
      { time: '2026-10-14T09:29:30Z', operation: 'ai.ask', trigger: 'toolbar', window: 'main', outcome: 'error', errorCode: 'ai-engine-exited' },
    ],
    snapshots: [
      {
        id: '20261014T092941000-00000001',
        createdAt: '2026-10-14T09:29:41Z',
        reason: 'engine-error',
        message: 'The AI engine stopped unexpectedly (exit code 3)',
        view: 'AI chat',
        mime: 'image/png',
        bytes: 68,
        steps: ['1. Opened <dir#1a2b3c4d>/samples (3 files) from the menu', "2. Clicked 'Ask the assistant' from the toolbar"],
        breadcrumbs: [],
      },
    ],
    sessionRecovery: null,
    description: 'The assistant stopped answering after my second question.',
    unreadable: [],
  };
}

/** What `diagnostics_report` answers in the mock: the sample, when a diagnostics zip is among the sources. */
export function sampleReportState(sources: { path: string; name: string }[]): DiagnosticsReportState {
  const zip = sources.find((s) => DIAGNOSTICS_ZIP.test(s.path) || DIAGNOSTICS_ZIP.test(s.name));
  if (!zip) return { report: null, notice: null };
  return { report: sampleReport(zip.name || zip.path.split(/[\\/]/).pop() || zip.path), notice: null };
}
