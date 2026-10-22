/**
 * Every mock backend of the browser build (dev server, e2e tests), in one module that is only ever loaded dynamically
 * (`loadBrowserMocks`, before bootstrapping outside Tauri): the desktop app never loads or parses mock code. Provider
 * factories reach these through `browserMocks()`; add a new mock backend here rather than importing it statically.
 */
export { MockAnalysisBackend, recordsFrom } from './analysis/mock/mock-analysis-backend';
export { MockConfigDiffBackend } from './config-diff/mock/mock-config-diff-backend';
export { MOCK_DUMP_PATHS, MOCK_UMDH_PATHS, MockDumpsBackend } from './dumps/mock/mock-dumps-backend';
export { MockBackend } from './core/mock/mock-backend';
export { mockOptionsFromSearch } from './core/mock/mock-options';
export { MOCK_CAPTURE_PATHS } from './core/mock/mock-profiling-data';
export { MockProfilingBackend } from './core/mock/mock-profiling';
export { MockUxBackend } from './core/mock/mock-ux-backend';
export { MockSnapshotBackend } from './diagnostics/mock/mock-snapshot-backend';
export { MockSchedulesBackend } from './schedules/mock/mock-schedules-backend';
export { MockSettingsBackend } from './settings/mock-settings-backend';
export { MockStorageBackend } from './settings/storage/mock-storage-backend';
export { MockParsersBackend } from './settings/parsers/mock-parsers-backend';
export { MockKnowledgeBackend } from './ai/knowledge/mock-knowledge-backend';
export { MockNotesBackend } from './notes/mock/mock-notes-backend';
export { MockProblemsBackend } from './problems/mock/mock-problems-backend';
export { MockStorePaneBackend } from './store-pane/mock/mock-store-pane-backend';
export { MockShareBackend } from './share/mock/mock-share-backend';
export { MockMonitorCharts, mockMonitorChartsOptions } from './tools/monitor/mock/mock-monitor-charts';
export { MockPluginPackagesBackend } from './plugins-panel/mock/mock-plugin-packages-backend';
export { MockToolsBackend } from './tools/mock/mock-tools-backend';
export { MockUpdatesBackend } from './updates/mock-updates-backend';
export { MockWindowingBackend } from './windowing/mock-windowing-backend';
export { MockRecentBackend } from './core/recent/mock-recent-backend';
