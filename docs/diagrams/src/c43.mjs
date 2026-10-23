import { P, save, link, hub, grp, bottom, Y0 } from './common.mjs';
const nm = { core: 'Engine component (pure)', accent: 'Port or adapter', process: 'Process · data', external: 'External system', plugin: 'Plugin', ui: 'UI component' };
const leg = (d, roles) => d.legend([...roles.map((k) => ({ kind: k, label: nm[k] })), { kind: 'line', label: 'Uses' }]);
const C = (id, name, tech, desc, role = 'core') => [id, role, { name, tech, desc }];
const GX = (i, nw = 232, gap = 96) => 48 + i * (nw + 48 + gap);
const gb = (d, id) => d.rect[id][1] + d.rect[id][3];

// ---------- engine 1: ingest ----------
{
  const d = P({ title: 'debrief-core — ingest', kind: 'C4 · Level 3 — Components of debrief-core (1 of 4)', w: 1472,
    desc: 'From a file on disk to events in the store: read, ingest, interpret with the profile, hand to the Store ports.' });
  grp(d, 'g1', GX(0), Y0, 'Read files', [
    C('arc', 'archive', 'zip · tar · gzip', 'Expands archives under size and entry limits.'),
    C('imp', 'importers', '`Importer` trait, `Importers`', 'Built in: text, JSON, CLEF, CSV, video frames. The rest are plugins.'),
    C('par', 'parser, survey', '`ParserSet`', 'A TOML parser library matched per file; the survey proposes parsers.'),
  ]);
  grp(d, 'g2', GX(1), Y0, 'Ingest', [
    C('ing', 'ingest', '`FilePlan`, `RecordBatcher`', 'Streams 8 MiB chunks into event batches; every failure lists the file as unrecognised.'),
    C('enc', 'encoding, paste', 'BOM · chardetng', 'Detects text encodings; names and bounds pasted text.'),
    C('win', 'import window', '`ImportWindow` (ADR 0047)', 'Drops events outside a time range before the pipeline.'),
  ]);
  grp(d, 'g3', GX(2), Y0, 'Interpret', [
    C('pro', 'profile', '`CompiledProfile`', 'Extractors, markers, lenses, charts, routes and spans from `default_profile.toml`.'),
    C('pip', 'pipeline, script', '`Pipeline`, Rhai', 'Drop, keep, dedupe, enrich; sandboxed scripts.'),
    C('ske', 'skew', 'Clock offsets', 'Per-file offsets applied to stored events (ADR 0006, 0037).'),
  ]);
  grp(d, 'g4', GX(3), Y0, 'Store ports', [
    C('por', 'ports', '`debrief_core::ports`', 'Fourteen traits combined as `Store`: `Ingest`, `Profiles`, `Skew`, `Tail` and the readers.', 'accent'),
    C('bat', 'debrief-batch', '`EventBatch`', 'The Arrow batch that flows to the writer and the index.'),
  ]);
  const y2 = Math.max(gb(d, 'g1'), gb(d, 'g2'), gb(d, 'g3'), gb(d, 'g4')) + 56;
  grp(d, 'g5', GX(0), y2, 'Plugin runtime', [
    C('reg', 'plugin', '`PluginRegistry`, `Guarded*`', 'Wraps every plugin contribution; panics become errors, repeat offenders are quarantined.'),
  ]);
  grp(d, 'g6', GX(3), y2, 'Watched folders', [
    C('wat', 'watch', '`WatchState`', 'Pure state machine: a file must settle, retries back off, known issues are announced once.'),
  ]);
  link(d, 'g1', 'R', 'g2', 'L', { label: 'Feeds' });
  link(d, 'g2', 'R', 'g3', 'L', { label: 'Applies' });
  link(d, 'g3', 'R', 'g4', 'L', { label: 'Writes' });
  link(d, 'g5', 'T', 'g1', 'B', { label: 'Contributes' });
  link(d, 'g6', 'T', 'g4', 'B', { label: 'Opens' });
  leg(d, ['core', 'accent']);
  save('c4-3-engine-ingest', d, Math.max(gb(d, 'g5'), gb(d, 'g6')));
}

// ---------- engine 2: query and aggregation ----------
{
  const d = P({ title: 'debrief-core — query and aggregation', kind: 'C4 · Level 3 — Components of debrief-core (2 of 4)', w: 1344,
    desc: 'Filters, the query language and aggregates. Every view reads through the ports; no view holds records.' });
  grp(d, 'g1', GX(0), Y0, 'Views over the ports', [
    C('hom', 'home', '`HomeSummary`', 'Counts, history by level, markers and anomalies from store queries.'),
    C('sea', 'search_all', 'Search everything', 'Events, sources, fields, markers, notes, knowledge; bounded and cancellable.'),
    C('len', 'lens', '`LensCfg` (ADR 0049)', 'Named view presets of a profile, validated when it compiles.'),
    C('liv', 'live', '`LiveSession`', 'Alert rules, silence and per-file tail status.'),
  ]);
  grp(d, 'g3', GX(2), Y0, 'Query', [
    C('ql', 'ql', 'Query language', 'Parser, executor, `stats`, “did you mean”, completion.'),
    C('qry', 'query, condition', '`CompiledQuery`, `Condition`', 'Filters shared by profiles, lenses and queries.'),
    C('rx', 'user_regex', 'Bounded regex', 'Caps pattern length and compiled size.'),
  ]);
  grp(d, 'g4', GX(2), bottom(d, 'g3') + 24 + 56, 'Aggregate', [
    C('cha', 'chart, spans', '`ChartCfg`, `[[span]]`', 'Timeline and chart buckets; start/stop events paired into runs.'),
  ]);
  grp(d, 'g2', GX(1), Y0, 'Store ports (read)', [
    C('por', 'ports', '`Query`, `Positions`, `Search`', 'Pages, timeline, facets, charts, field statistics, source context, `run_query_with`, `check_query`.', 'accent'),
  ], { minH: bottom(d, 'g4') - Y0 });
  link(d, 'g1', 'R', 'g2', 'L', { label: 'Reads' });
  link(d, 'g2', 'R', 'g3', 'L', { label: 'Filters by' });
  link(d, 'g2', 'R', 'g4', 'L', { label: 'Aggregates' });
  leg(d, ['core', 'accent']);
  save('c4-3-engine-query', d, bottom(d, 'g4'));
}

// ---------- engine 3: analysis and investigation ----------
{
  const d = P({ title: 'debrief-core — analysis and investigation', kind: 'C4 · Level 3 — Components of debrief-core (3 of 4)', w: 1344,
    desc: 'Analyses are functions over the Query port or implementations of the Analysis port; each result says how much data it covers.' });
  grp(d, 'g1', GX(0), Y0, 'Investigate', [
    C('kin', 'known_issues', 'Known-issue blocks', 'Declared errors matched by running their filter through the store.'),
    C('inc', 'incident', '`build_incident_report`', 'The incident summary as an ordinary `Report`; the AI narrative is separate.'),
    C('pla', 'playbook', '`Playbook`, `PlaybookRun`', 'Versioned JSON steps and recorded runs (ADR 0039).'),
    C('rel', 'related', '`RelatedSource`', 'Finds the events that belong to a screenshot.'),
  ]);
  grp(d, 'g2', GX(1), Y0, 'Analysis', [
    C('ano', 'anomaly, trend', '`analysis::anomaly`', 'Spikes, silences and rate changes per instance.'),
    C('cmp', 'compare, moment', '`analysis::moment`', 'Window comparison and the Moment summary.'),
    C('out', 'outliers', '`error_outliers` (ADR 0045)', 'Values over-represented among errors; pure over the `Query` port.'),
    C('pat', 'patterns, exceptions', '`Pattern`, `ExceptionGroup`', 'Message templates and stack traces grouped by signature.'),
  ]);
  grp(d, 'g3', GX(2), Y0, 'Store ports', [
    C('por', 'ports', '`Analysis`, `Query`', 'Patterns, exceptions, anomalies, compare, correlate, traces, requests, moment, overview, runs. The DuckDB store and the in-memory `Workspace` oracle implement the same traits.', 'accent'),
  ], { minH: bottom(d, 'pat') + 24 - Y0 });
  link(d, 'g1', 'R', 'g2', 'L', { label: 'Summarises' });
  link(d, 'g2', 'R', 'g3', 'L', { label: 'Reads' });
  leg(d, ['core', 'accent']);
  save('c4-3-engine-analysis', d, bottom(d, 'g1'));
}

// ---------- engine 4: sharing and projects ----------
{
  const d = P({ title: 'debrief-core — sharing, projects and settings', kind: 'C4 · Level 3 — Components of debrief-core (4 of 4)', w: 1504,
    desc: 'What leaves the app or is saved: exports, reports, bundles, projects and dashboards. One redactor serves them all.' });
  d.node('pdf', 72, Y0 + 40, 232, null, 'accent', { name: 'debrief-pdf', tech: 'Rust library, adapter', desc: 'PDF reports behind the `ReportRenderer` port.' });
  grp(d, 'g1', GX(1), Y0, 'Share', [
    C('exp', 'export', 'CSV · CLEF · JSON', 'Exports of the filtered events.'),
    C('rep', 'report', '`Report`, `ReportRenderer`', 'Versioned report model in Markdown and HTML (ADR 0007).'),
    C('red', 'redact', '`Redactor`, `redact::copy`', 'One redactor for the view, exports, reports, bundles and the redacted copy.'),
  ]);
  grp(d, 'g2', GX(2), Y0, 'Projects', [
    C('prj', 'project, package', '`ProjectManifest`', 'Session files and the manifest of a `.debriefz` project.'),
    C('bun', 'bundle', '`.debriefz`', 'Reads and writes bundles with archive limits and hashes.'),
    C('das', 'dashboard, tiles', '`DashboardCfg`', 'Per-profile tile grids.'),
  ]);
  grp(d, 'g3', GX(3), Y0, 'Configuration', [
    C('cfg', 'config', 'Config trees', 'Configuration snapshots, parsers and diff.'),
    C('set', 'settings, template', 'Typed settings', 'Schema and validation; ready-made profiles; no silent overwrite.'),
    C('svg', 'svg, icons', 'SVG sanitiser', 'Untrusted SVG is rebuilt from an allowlist.'),
  ]);
  const y2 = Math.max(bottom(d, 'g1'), bottom(d, 'g2'), bottom(d, 'g3')) + 24;
  const gw = 3 * 280 + 2 * 96;
  d.node('por', GX(1), y2 + 56, gw, 80, 'accent', { name: 'Store ports', tech: '`Redaction`, `Projects`, `Query`', desc: 'Everything above reads or writes the store only through these traits.' });
  link(d, 'pdf', 'R', 'g1', 'L', { dash: true, arrow: 'open', label: 'Implements' });
  for (const g of ['g1', 'g2', 'g3']) link(d, g, 'B', 'por', 'T', { t: [0.5, (d.rect[g][0] + d.rect[g][2] / 2 - d.rect.por[0]) / d.rect.por[2]] });
  leg(d, ['core', 'accent']);
  save('c4-3-engine-sharing', d, y2 + 56 + 80);
}

// ---------- UI 1: frame, state, backends ----------
{
  const U = (id, name, tech, desc) => [id, 'ui', { name, tech, desc }];
  const d = P({ title: 'Angular UI — frame, state and backends', kind: 'C4 · Level 3 — Components of the Angular UI (1 of 3)', w: 1504,
    desc: 'Folders under `app/src/app`. Components never call Tauri: they inject the backend token and read SignalStores.' });
  grp(d, 'g1', GX(0), Y0, 'App frame', [
    U('shell', 'shell, windowing', 'Title bar, menus, status bar', 'Activity bar, native or in-window menus (ADR 0008), windows and the tray.'),
    U('dock', 'dock, popout', 'dockview', 'VS-style docking, named layouts, auto-hide, panels in OS windows.'),
    U('ext', 'extensions, builtin, palette', '`DEBRIEF_VIEWS`, commands', 'View and command registries, keybindings and the command palette.'),
    U('ui', 'ui, shared', 'Angular CDK + Material', 'Dialogs, context menus, `debrief-icon`, switches, overwrite guard.'),
  ]);
  grp(d, 'g2', GX(1), Y0, 'State', [
    U('wss', 'WorkspaceStore', 'SignalStore', 'Sources, summary, selection and the data version that views watch.'),
    U('qs', 'QueryStore', '`effectiveQuery`', 'The filter bar’s query plus enabled filter sets; every view sends it to the engine.'),
    U('prf', 'ui-prefs, theme', 'SignalStores', 'Layout, preferences and the three themes (Calm, Console, Precision).'),
    U('log', 'logger, failure-reporter', 'Diagnostics', 'UI log to the app log; inline errors with Retry.'),
  ]);
  grp(d, 'g3', GX(2), Y0, 'Backends', [
    U('be', 'DebriefBackend', 'DI token, abstract class', 'Typed by `api.types.ts` (camelCase JSON); small domain backends per area.'),
    U('tb', 'TauriBackend', '`invoke()`, `listen()`', 'The only place that calls Tauri.'),
    U('mb', 'MockBackend', '`core/mock`', 'Plausible data for browser development and UI tests; never the reference.'),
  ]);
  d.node('rs', GX(3), d.rect.tb[1], 280, d.rect.tb[3], 'accent', { name: 'Tauri shell', tech: 'Rust, `app/src-tauri`', desc: 'Validates every command; calls the Store ports.' });
  link(d, 'g1', 'R', 'g2', 'L', { label: 'Injects' });
  link(d, 'g2', 'R', 'g3', 'L', { label: 'Calls' });
  link(d, 'tb', 'R', 'rs', 'L', { label: 'Invokes' });
  d.legend([{ kind: 'ui', label: 'UI component (folder)' }, { kind: 'accent', label: 'Container' }, { kind: 'line', label: 'Uses' }]);
  save('c4-3-ui-frame', d, Math.max(bottom(d, 'g1'), bottom(d, 'g2'), bottom(d, 'g3')));
}

// ---------- UI 2: investigating ----------
{
  const U = (id, name, tech, desc) => [id, 'ui', { name, tech, desc }];
  const d = P({ title: 'Angular UI — investigating', kind: 'C4 · Level 3 — Components of the Angular UI (2 of 3)', w: 1504,
    desc: 'The views an analyst lives in. Each folder holds its views and SignalStores and talks to the engine through the backends.' });
  grp(d, 'g1', GX(0), Y0, 'Look at events', [
    U('grid', 'grid, inspector, workspace', 'Virtualised grid', 'Paged events, inspector, facets, fields and the filter bar.'),
    U('tl', 'timeline, markers', 'Custom timeline', 'Lanes per instance, markers, annotations, overview.'),
    U('met', 'metrics, context', 'ECharts', 'Charts, Moment and the source lines around an event.'),
  ]);
  grp(d, 'g2', GX(1), Y0, 'Find and narrow', [
    U('len', 'lenses', '`LensStore`, `FilterSetStore`', 'Built-in and saved lenses; filter sets on Alt+1 to Alt+9 (ADR 0049).'),
    U('sea', 'search-everything', 'Palette', 'Events, sources, fields, markers, notes, knowledge; opens the match.'),
    U('hom', 'home', 'Dashboard', 'Live tiles, history by level, health check and quick actions.'),
  ]);
  grp(d, 'g3', GX(2), Y0, 'Keep up', [
    U('liv', 'live', 'Live tail', 'Tail status, alerts and follow mode.'),
    U('wat', 'watched', '`WatchedFoldersStore`', 'Folders imported as files settle; a new known issue is announced (ADR 0048).'),
    U('job', 'jobs, notifications', 'Jobs pane', 'Every long task as a job with Cancel and Retry; the notification centre.'),
  ]);
  grp(d, 'g4', GX(3), Y0, 'Understand', [
    U('ana', 'analysis', 'Anomalies, patterns', 'Anomalies, patterns, exceptions, error outliers and comparisons.'),
    U('inv', 'investigate', 'Known issues, incident', 'Declared errors, the one-click incident summary, playbook runs.'),
    U('not', 'notes', 'Notes', 'Notes anchored to events, times, sources and image regions.'),
  ]);
  const y2 = Math.max(...['g1', 'g2', 'g3', 'g4'].map((g) => bottom(d, g))) + 56;
  const gw = 4 * 280 + 3 * 96;
  d.node('bar', 48, y2, gw, 72, 'ui', { name: 'WorkspaceStore, QueryStore and the DebriefBackend token', tech: 'See “frame, state and backends”', desc: 'Views read signals and call the backend; none calls Tauri.' });
  for (const g of ['g1', 'g2', 'g3', 'g4']) link(d, g, 'B', 'bar', 'T', { t: [0.5, (d.rect[g][0] + d.rect[g][2] / 2 - 48) / gw] });
  d.legend([{ kind: 'ui', label: 'UI component (folder)' }, { kind: 'line', label: 'Uses' }]);
  save('c4-3-ui-investigate', d, y2 + 72);
}

// ---------- UI 3: building and tools ----------
{
  const U = (id, name, tech, desc) => [id, 'ui', { name, tech, desc }];
  const d = P({ title: 'Angular UI — building, sharing and tools', kind: 'C4 · Level 3 — Components of the Angular UI (3 of 3)', w: 1504,
    desc: 'Teaching Debrief a format, sharing results, the specialist views and the system panels.' });
  grp(d, 'g1', GX(0), Y0, 'Teach and shape', [
    U('pro', 'profile', 'Profile editor', 'Extractors, markers, charts and pipeline with a live preview.'),
    U('par', 'settings ▸ parsers', 'Parser library', 'Parsers and AI-assisted generation, checked by the engine.'),
    U('skw', 'skew, config-diff', 'Comparison', 'Clock-skew correction and configuration diff.'),
  ]);
  grp(d, 'g2', GX(1), Y0, 'Open and share', [
    U('imp', 'import, sources', 'Import', 'Progress with Cancel, a time-range import and per-source actions.'),
    U('shr', 'share, export', 'Reports and bundles', 'Incident reports, exports, redacted copies and `.debriefz` bundles.'),
    U('prj', 'projects, lifecycle', 'Projects', 'Save, open and recent projects; crash recovery and autosave.'),
  ]);
  grp(d, 'g3', GX(2), Y0, 'Specialist views', [
    U('prof', 'profiling, dumps', 'Profiler and dump views', 'Hotspots, flame graphs, heap snapshots, crash dumps and UMDH logs.'),
    U('vid', 'video, store-pane', 'Video and Store pane', 'Frame strip and alignment; store overview and a read-only SQL console.'),
    U('tools', 'tools, diagnostics, schedules', 'System tools', 'Performance Monitor, process monitor, Event Log, the agent.'),
  ]);
  grp(d, 'g4', GX(3), Y0, 'Assistants and system', [
    U('ai', 'ai', 'Local AI assistant', 'Chat with approvals, models, runtimes, vision, knowledge files.'),
    U('brw', 'browser', 'Research and Agent mode', 'The research dialog, the AI-browsing chip with Stop and the Chromium setting (ADR 0043).'),
    U('set', 'settings, plugins-panel, updates', 'Preferences', 'Settings, plugin install and health, updater, About.'),
  ]);
  const y2 = Math.max(...['g1', 'g2', 'g3', 'g4'].map((g) => bottom(d, g))) + 56;
  const gw = 4 * 280 + 3 * 96;
  d.node('bar', 48, y2, gw, 72, 'ui', { name: 'WorkspaceStore, QueryStore and the DebriefBackend token', tech: 'See “frame, state and backends”', desc: 'Views read signals and call the backend; none calls Tauri.' });
  for (const g of ['g1', 'g2', 'g3', 'g4']) link(d, g, 'B', 'bar', 'T', { t: [0.5, (d.rect[g][0] + d.rect[g][2] / 2 - 48) / gw] });
  d.legend([{ kind: 'ui', label: 'UI component (folder)' }, { kind: 'line', label: 'Uses' }]);
  save('c4-3-ui-tools', d, y2 + 72);
}

// ---------- research browser ----------
{
  const d = P({ title: 'Research browser — components', kind: 'C4 · Level 3 — Research and Agent mode over a real Chromium', w: 1504,
    desc: 'The AI sees and drives a page only through three ports. A neutral driver crate speaks DevTools; every indicator that the AI is browsing lives outside the page.' });
  grp(d, 'g1', GX(0), Y0, 'debrief-ai: browser ports and rules', [
    C('por', 'ports', '`PageReader`, `ResultsReader`, `PageDriver`', 'What the AI may ask of a page; the same page scripts as before.', 'accent'),
    C('pol', 'policy, leak guard', '`check_query`, site rules', 'Checks a search for workspace data; site rules and approvals.'),
    C('gate', 'ReplyGate', 'Nonce · origin · size', 'Every reply from a page passes a one-shot gate.'),
  ]);
  grp(d, 'g2', GX(1), Y0, 'Tauri shell: browser/', [
    ['ren', 'accent', { name: 'renderer', tech: '`ChromiumPage`, `Renderers`', desc: 'Implements the ports over the driver; one Chromium per window, on demand.' }],
    ['res', 'accent', { name: 'research, driving', tech: 'Jobs, `BrowserHub`', desc: 'Research jobs, chat turns and the status chip with Stop; indicators stay outside the page.' }],
    ['run', 'accent', { name: 'runtime', tech: 'Chrome for Testing', desc: 'Optional pinned download with SHA-256, self-test and rollback by never switching.' }],
  ]);
  grp(d, 'g3', GX(2), Y0, 'chromium-driver', [
    C('dis', 'discover, install', 'Edge · Chrome · Chromium', 'Finds a browser (`CHROMIUM_PATH` wins); verified, zip-slip-safe install.'),
    C('lau', 'launch, process', 'Supervised child', 'Own process group, killed on drop, time cap, throwaway profile, off screen.'),
    C('cdp', 'cdp, page', 'DevTools client', 'Minimal synchronous client (loopback only); navigate, evaluate, history.'),
    C('rdy', 'ready, challenge, cue', 'Pure rules', 'Readiness, CAPTCHA and consent-wall detection, window title cues.'),
  ]);
  d.node('chr', GX(3), d.rect.lau[1], 280, d.rect.lau[3], 'process', { name: 'Chromium', tech: 'Child process', desc: 'Real user agent; Chrome’s own automation banner stays on.' });
  d.node('web', GX(3), d.rect.chr[1] + d.rect.chr[3] + 56, 280, null, 'external', { name: 'Web sites', tech: 'Pages the user allows', desc: 'Search results and the pages read for research.' });
  d.node('cft', GX(1) + 24, bottom(d, 'g2') + 56, 232, null, 'external', { name: 'Chrome for Testing', tech: 'Download host', desc: 'Pinned version per platform.' });
  link(d, 'g2', 'L', 'g1', 'R', { dash: true, arrow: 'open', label: 'Implements' });
  link(d, 'g2', 'R', 'g3', 'L', { label: 'Drives' });
  link(d, 'lau', 'R', 'chr', 'L', { label: 'Starts' });
  link(d, 'chr', 'B', 'web', 'T', { label: 'Loads' });
  link(d, 'run', 'B', 'cft', 'T', { label: 'Downloads' });
  d.legend([{ kind: 'accent', label: 'Debrief container' }, { kind: 'core', label: 'Component' }, { kind: 'process', label: 'Process' }, { kind: 'external', label: 'External system' }, { kind: 'line', label: 'Uses' }, { kind: 'dashline', label: 'Implements' }]);
  save('c4-3-research-browser', d, Math.max(bottom(d, 'g1'), bottom(d, 'cft'), bottom(d, 'g3')));
}
