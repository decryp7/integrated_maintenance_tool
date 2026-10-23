import { P, save, link, hub, stack, row, bottom, right, legend, Y0 } from './common.mjs';

// ---------- C4 level 1 ----------
{
  const d = P({ title: 'Debrief — system context', kind: 'C4 · Level 1 — System context', w: 1344,
    desc: 'Debrief runs on one machine and reads what the user opens. The network is used only for an update, a download or web research the user starts.' });
  const W = 288;
  const people = [
    ['sup', 'process', { name: 'Support engineer', tech: 'Person', desc: 'Opens customer logs and bundles and finds the cause of an incident.', person: true }],
    ['dev', 'process', { name: 'Developer', tech: 'Person', desc: 'Analyses logs and captures; writes parsers, profiles and pipelines.', person: true }],
    ['adm', 'process', { name: 'Administrator', tech: 'Person', desc: 'Schedules collection, watches folders and gathers diagnostics from servers.', person: true }],
    ['aut', 'process', { name: 'Plugin author', tech: 'Person', desc: 'Extends Debrief with Rust plugins; ships signed packages.', person: true }],
  ];
  const ext = [
    ['fs', 'external', { name: 'File system', tech: 'Local and network drives', desc: 'Logs, archives, captures, dumps, projects (`.debriefz`) and bundles.' }],
    ['wv', 'external', { name: 'OS webview', tech: 'WebView2 · WKWebView · WebKitGTK', desc: 'Renders Debrief’s user interface.' }],
    ['win', 'external', { name: 'Windows diagnostics', tech: 'Event Log · PDH · logman', desc: 'Live channels, counters and collector sets.' }],
    ['plg', 'external', { name: 'Third-party plugins', tech: 'Signed `.debriefplugin` packages', desc: 'Importers, transforms and tools in sandboxed Rust processes.' }],
    ['chr', 'external', { name: 'Chromium browser', tech: 'Edge · Chrome · Chromium', desc: 'Driven over DevTools for research; shows the web sites.' }],
    ['dl', 'external', { name: 'Download hosts', tech: 'Gitea · Hugging Face · GitHub', desc: 'Signed updates, AI models, `llama-server`, FFmpeg.' }],
  ];
  const ph = Math.max(...people.map(([, , o]) => d.fit(W, o))), eh = Math.max(...ext.map(([, , o]) => d.fit(W, o)));
  people.forEach(([id, r, o], i) => d.node(id, 48, Y0 + i * (ph + 40), W, ph, r, o));
  ext.forEach(([id, r, o], i) => d.node(id, 48 + W + 192 + W + 192, Y0 + i * (eh + 40), W, eh, r, o));
  const total = 6 * eh + 5 * 40;
  d.node('sys', 48 + W + 192, Y0, W, total, 'accent', { name: 'Debrief', tech: 'Desktop app: Rust engine + Tauri 2 + Angular 22',
    desc: 'Imports logs, event logs, counters, dumps and profiler captures once into a DuckDB store and correlates them on one timeline. Live tail, watched folders, error outliers, known issues and incident reports, a command line for scripts, and an optional local AI assistant. Runs offline.' });
  const lab = ['Investigates', 'Writes profiles', 'Collects, schedules', 'Builds plugins'];
  people.forEach(([id], i) => hub(d, 'sys', 'L', id, 'R', { rev: true, label: lab[i] }));
  const rl = ['Reads, watches', 'Renders in', 'Follows, samples', 'Calls', 'Drives', 'Downloads'];
  ext.forEach(([id], i) => hub(d, 'sys', 'R', id, 'L', { label: rl[i] }));
  legend(d, ['process', 'accent', 'external']);
  save('c4-1-context', d, Y0 + total);
}

// ---------- C4 level 2: the workbench ----------
{
  const d = P({ title: 'Debrief — containers: the workbench', kind: 'C4 · Level 2 — Containers (1 of 3)', w: 1568,
    desc: 'One app process per instance, a store per window. The UI, shell and libraries share the process; the store and the plugins run in sandboxed child processes.' });
  const NW = 264, RH = 128;
  d.node('user', 72, Y0, NW, null, 'process', { name: 'Support engineer / Developer', tech: 'Person', desc: 'Investigates incidents; writes parsers and profiles.', person: true });
  const gy = Y0 + d.rect.user[3] + 56;
  const gx = 48, ux = gx + 24, sx = ux + NW + 112, cx = sx + 280 + 112;
  const ry = (i) => gy + 40 + i * (RH + 40);
  const gh = 40 + 3 * RH + 2 * 40 + 24;
  d.group('app', gx, gy, 24 + NW + 112 + 280 + 112 + NW + 24, gh, 'App process [Tauri 2]');
  d.node('ui', ux, ry(0), NW, RH, 'ui', { name: 'Angular UI', tech: 'Angular 22 · SignalStores · ECharts · dockview', desc: 'Reaches the engine only through `DebriefBackend`.' });
  d.node('shell', sx, ry(0), 280, 3 * RH + 80, 'accent', { name: 'Tauri shell', tech: 'app/src-tauri: Rust, Tauri 2',
    desc: 'Validates every IPC argument, then calls the Store ports. Windows, tray, sessions, settings, updater, jobs and notifications, watched folders, live tail, the research browser and AI hosting.' });
  d.node('host', cx, ry(0), NW, RH, 'accent', { name: 'debrief-plugin-host', tech: 'Rust library, adapter', desc: 'Starts, sandboxes and supervises plugin processes; registers guarded proxies. Also linked into the store worker.' });
  d.node('core', cx, ry(1), NW, RH, 'core', { name: 'debrief-core', tech: 'Rust library, the engine', desc: 'Store and Importer ports, parsers, profiles, query language, analysis, reports.' });
  d.node('client', cx, ry(2), NW, RH, 'accent', { name: 'debrief-store-client', tech: 'Rust library, no database linked', desc: '`RemoteStore`: the Store ports over `debrief-ipc`.' });
  // child processes
  const wx = gx + 24 + NW + 112 + 280 + 112 + NW + 24 + 136, wgw = 24 + 288 + 24;
  d.group('kids', wx, gy, wgw, gh, 'Child processes [sandboxed · supervised · debrief-ipc]', { os: true });
  d.node('plugins', wx + 24, ry(0), 288, RH, 'plugin', { name: 'debrief-plugins', tech: 'Rust, one process per built-in plugin', desc: 'Formats, Windows, web, OpenTelemetry, syslog, tools, profiling, dumps.' });
  d.node('third', wx + 24, ry(1), 288, RH, 'plugin', { name: 'Third-party plugins', tech: 'Rust, signed `.debriefplugin`', desc: 'Strictest sandbox; only the permissions the user granted.' });
  d.node('worker', wx + 24, ry(2), 288, RH, 'accent', { name: 'debrief-store-worker', tech: 'Rust + DuckDB + Tantivy', desc: 'Hosts every window’s store; Parquet segments, catalog, full-text index.' });
  d.node('fs', wx + 24, gy + gh + 56, 288, null, 'external', { name: 'File system', tech: 'Local and network drives', desc: 'Logs, store folders, `.debriefz` projects, settings.' });
  hub(d, 'shell', 'L', 'ui', 'R', { rev: true, label: 'Commands' });
  hub(d, 'shell', 'R', 'host', 'L', { label: 'Plugins' });
  hub(d, 'shell', 'R', 'core', 'L', { label: 'Ports' });
  hub(d, 'shell', 'R', 'client', 'L', { label: 'Store' });
  link(d, 'client', 'T', 'core', 'B', { label: 'Implements', at: 0.5 });
  link(d, 'host', 'B', 'core', 'T', { label: 'Registers' });
  link(d, 'host', 'R', 'plugins', 'L', { label: 'Runs' });
  link(d, 'client', 'R', 'worker', 'L', { label: 'Imports, queries' });
  link(d, 'worker', 'B', 'fs', 'T', { label: 'Reads, writes' });
  link(d, 'user', 'B', 'ui', 'T', { t: [0.85, 0.85], label: 'Uses', at: 0.3 });
  legend(d, ['process', 'ui', 'accent', 'core', 'plugin', 'external']);
  save('c4-2-containers', d, bottom(d, 'fs'));
}

// ---------- C4 level 2: AI, research browser, video ----------
{
  const d = P({ title: 'Debrief — containers: AI, research and video', kind: 'C4 · Level 2 — Containers (2 of 3)', w: 1568,
    desc: 'Libraries in the app process drive helper programs in sandboxed or supervised child processes. Everything is optional and off until the user asks.' });
  const NW = 272, RH = 112, gx = 48, sx = 72, lx = sx + 264 + 104;
  const ry = (i) => Y0 + 40 + i * (RH + 40);
  const gh = 40 + 4 * RH + 3 * 40 + 24;
  d.group('app', gx, Y0, 24 + 264 + 104 + NW + 24, gh, 'App process [Tauri 2]');
  d.node('shell', sx, ry(0), 264, 4 * RH + 3 * 40, 'accent', { name: 'Tauri shell', tech: 'app/src-tauri: Rust, Tauri 2',
    desc: 'Runs AI chat, research and video as jobs. Validates every request; asks before the AI does anything. Posts progress to the Jobs pane.' });
  d.node('ai', lx, ry(0), NW, RH, 'accent', { name: 'debrief-ai', tech: 'Rust library', desc: 'Worker supervisor, model and runtime downloads, vision, knowledge files, the browser rules.' });
  d.node('drv', lx, ry(1), NW, RH, 'accent', { name: 'chromium-driver', tech: 'Rust library, no Debrief types', desc: 'Finds a Chromium, launches it supervised and speaks DevTools to it.' });
  d.node('vid', lx, ry(2), NW, RH, 'accent', { name: 'debrief-video', tech: 'Rust library', desc: 'Frame selection and alignment; runs FFmpeg sandboxed.' });
  d.node('sym', lx, ry(3), NW, RH, 'accent', { name: 'debrief-symbols', tech: 'Rust library', desc: 'Matches PDBs by identity; fetches them into a bounded cache.' });
  const kx = lx + NW + 24 + 112, kw = 24 + 272 + 24;
  d.group('kids', kx, Y0, kw, gh, 'Child processes [sandboxed or supervised]', { os: true });
  d.node('wrk', kx + 24, ry(0), 272, RH, 'accent', { name: 'debrief-ai-worker', tech: 'Rust, sandboxed', desc: 'Inference worker; supervises `llama-server`.' });
  d.node('chr', kx + 24, ry(1), 272, RH, 'process', { name: 'Chromium', tech: 'Edge · Chrome · Chromium, off screen by default', desc: 'One per window, started on demand; killed when Debrief exits.' });
  d.node('ffm', kx + 24, ry(2), 272, RH, 'process', { name: 'ffmpeg, ffprobe', tech: 'Pinned LGPL runtime, downloaded', desc: 'No network, empty environment, memory-capped.' });
  const ex = kx + kw + 112;
  d.node('llm', ex, ry(0), 256, RH, 'process', { name: 'llama-server', tech: 'Prebuilt llama.cpp, downloaded', desc: 'Runs the GGUF model and its vision projector.' });
  d.node('web', ex, ry(1), 256, RH, 'external', { name: 'Web sites', tech: 'Any host the user allows', desc: 'Pages read for research or Agent mode.' });
  d.node('pdb', ex, ry(3), 256, RH, 'external', { name: 'Symbol servers', tech: 'Opt-in, off by default', desc: 'PDB files for crash dumps.' });
  hub(d, 'shell', 'R', 'ai', 'L', { label: 'Asks' });
  hub(d, 'shell', 'R', 'drv', 'L', { label: 'Browses' });
  hub(d, 'shell', 'R', 'vid', 'L', { label: 'Analyses' });
  hub(d, 'shell', 'R', 'sym', 'L', { label: 'Resolves' });
  link(d, 'ai', 'R', 'wrk', 'L', { label: 'Streams' });
  link(d, 'drv', 'R', 'chr', 'L', { label: 'DevTools' });
  link(d, 'vid', 'R', 'ffm', 'L', { label: 'Runs' });
  link(d, 'wrk', 'R', 'llm', 'L', { label: 'Completes' });
  link(d, 'chr', 'R', 'web', 'L', { label: 'Loads' });
  d.edge('sym', 'pdb', d.port('sym', 'R', 0.5), d.port('pdb', 'L', 0.5), { via: [] });
  d.lbl('Downloads', ex - 56, ry(3) + RH / 2, 'a');
  legend(d, ['accent', 'process', 'external']);
  save('c4-2-containers-helpers', d, Y0 + gh);
}

// ---------- C4 level 2: headless tools ----------
{
  const d = P({ title: 'Debrief — containers: command line and collection', kind: 'C4 · Level 2 — Containers (3 of 3)', w: 1344,
    desc: 'Scripts, CI and servers use the same engine without the app: the `debrief` command line over the store worker, and the collection tools that write what the app opens.' });
  const NW = 280, RH = 112;
  const ry = (i) => Y0 + 40 + i * (RH + 40);
  const gh = 40 + 4 * RH + 3 * 40 + 24;
  d.node('ci', 48, ry(0), NW, RH, 'external', { name: 'Scripts and CI', tech: 'Shell, pipelines', desc: 'Run `debrief` and read its exit code or JSON.' });
  d.node('adm', 48, ry(1), NW, 3 * RH + 80, 'process', { name: 'Administrator', tech: 'Person', desc: 'Installs the agent on servers; runs collection and monitoring by hand.', person: true });
  const gx = 48 + NW + 128, cw = 24 + NW + 24;
  d.group('tools', gx, Y0, cw, gh, 'Command-line tools [Rust · one process each]');
  d.node('cli', gx + 24, ry(0), NW, RH, 'accent', { name: 'debrief', tech: 'debrief-cli: import, query, analyze, watch, report', desc: 'Exit 0 done, 1 failed, 2 usage, 3 findings. `--json` prints one object.' });
  d.node('agent', gx + 24, ry(1), NW, RH, 'accent', { name: 'debrief-agent', tech: 'Rust CLI or OS service', desc: 'Scheduled and triggered collection on a server, with retention.' });
  d.node('coll', gx + 24, ry(2), NW, RH, 'accent', { name: 'debrief-collect', tech: 'Rust CLI and library', desc: 'Diagnostics bundle: a redacted `.debriefz`.' });
  d.node('mon', gx + 24, ry(3), NW, RH, 'accent', { name: 'debrief-monitor', tech: 'Rust CLI and library', desc: 'Process and counter sampling into rolling CLEF.' });
  const wx = gx + cw + 128;
  d.group('wg', wx, Y0, cw, 40 + RH + 24, 'Child process [supervised]', { os: true });
  d.node('wrk', wx + 24, ry(0), NW, RH, 'accent', { name: 'debrief-store-worker', tech: 'Rust + DuckDB + Tantivy', desc: 'The worker the app starts: same plugins, caps and store.' });
  d.node('fs', wx, ry(1), cw, 3 * RH + 80, 'process', { name: 'File system', tech: 'Watched folders · run folders · `.debriefz` · CLEF', desc: 'Where files settle and bundles are written. `debrief watch` imports what settles; the app opens the bundles.' });
  link(d, 'ci', 'R', 'cli', 'L', { label: 'Runs' });
  hub(d, 'adm', 'R', 'agent', 'L', { label: 'Installs' });
  hub(d, 'adm', 'R', 'coll', 'L', { label: 'Runs' });
  hub(d, 'adm', 'R', 'mon', 'L', { label: 'Runs' });
  link(d, 'cli', 'R', 'wrk', 'L', { label: 'Imports, queries' });
  hub(d, 'fs', 'L', 'agent', 'R', { rev: true, label: 'Writes' });
  hub(d, 'fs', 'L', 'coll', 'R', { rev: true, label: 'Writes' });
  hub(d, 'fs', 'L', 'mon', 'R', { rev: true, label: 'Writes' });
  legend(d, ['accent', 'process', 'external']);
  save('c4-2-containers-headless', d, Y0 + gh);
}
