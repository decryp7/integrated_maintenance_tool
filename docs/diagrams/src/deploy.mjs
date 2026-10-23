import { P, save, link, hub, grp, bottom, Y0 } from './common.mjs';

// ---------- workstation ----------
{
  const d = P({ title: 'Debrief — deployment on the analyst workstation', kind: 'UML · Deployment diagram (1 of 3)', w: 1520,
    desc: 'One app process per instance with a store per window. The store, the plugins, the AI and the helpers run in their own supervised processes.' });
  const NW = 264, RH = 112;
  const mx = 48, my = Y0, mw = 1424;
  const dataSpec = { name: 'App data folder', tech: '«folder» per user', desc: '`settings.json`, `sessions/`, `stores/` (a folder per window), parser library, profiles, notes, `ai/` models and runtimes, installed plugins, the app log.' };
  const ay0 = my + 48 + 40 + RH + 24 + 56, dy0 = ay0 + 40 + RH + 24 + 24 + 56;
  const mh = Math.max(dy0 + d.fit(24 + NW + 24, dataSpec) + 24 - my, 48 + 40 + 4 * RH + 3 * 24 + 24 + 24);
  d.group('m', mx, my, mw, mh, '«device» Analyst workstation [Windows 10 / 11 · macOS · Linux]');
  // left column
  d.group('wv', 72, my + 48, 24 + NW + 24, 40 + RH + 24, '«execution environment» OS webview');
  d.node('ui', 96, my + 48 + 40, NW, RH, 'ui', { name: 'Angular UI bundle', tech: '«artifact» HTML, JS, CSS in the app', desc: 'Strict CSP, no remote content; `ImtBackend` is `TauriBackend`.' });
  const ay = my + 48 + 40 + RH + 24 + 56;
  d.group('ap', 72, ay, 24 + NW + 24, 40 + RH + 24 + 24, '«execution environment» App process');
  d.node('app', 96, ay + 40, NW, RH + 24, 'accent', { name: 'imt-app', tech: '«executable» Rust, Tauri 2', desc: 'The shell with `imt-core`, `imt-store-client`, `imt-plugin-host`, `imt-ai`, `imt-video` and `chromium-driver` linked in; tray, single instance.' });
  const dy = ay + 40 + RH + 24 + 24 + 56;
  d.node('data', 72, dy, 24 + NW + 24, null, 'process', dataSpec);
  // children
  const kx = 72 + 24 + NW + 24 + 144, kw = mx + mw - 24 - kx;
  const cw = (kw - 24 * 2 - 24) / 2;
  const ky = my + 48;
  const rows = 4, kh = 40 + rows * RH + (rows - 1) * 24 + 24;
  d.group('kids', kx, ky, kw, kh, '«execution environment» Child processes [supervised: restarted with back-off, killed with the app]', { os: true });
  const cx = (i) => kx + 24 + i * (cw + 24), cy = (r) => ky + 40 + r * (RH + 24);
  d.node('pl', cx(0), cy(0), cw, RH, 'plugin', { name: 'imt-plugins', tech: '«executable» Rust, one process per built-in plugin', desc: 'Each plugin in its own sandbox: `--plugin <id>`.' });
  d.node('tp', cx(0), cy(1), cw, RH, 'plugin', { name: 'Third-party plugin', tech: '«executable» Rust, from a `.imtplugin`', desc: 'Signed package; strictest sandbox, only the granted permissions.' });
  d.node('sw', cx(0), cy(2), cw, RH, 'accent', { name: 'imt-store-worker', tech: '«executable» Rust + libduckdb', desc: 'DuckDB stores and Tantivy indexes of all windows; 2 GiB hard cap.' });
  d.node('ff', cx(0), cy(3), cw, RH, 'process', { name: 'ffmpeg, ffprobe', tech: '«executable» pinned LGPL runtime', desc: 'Frame passes for video sources: no network, memory-capped.' });
  d.node('aw', cx(1), cy(0), cw, RH, 'accent', { name: 'imt-ai-worker', tech: '«executable» Rust', desc: 'Inference worker; starts `llama-server`.' });
  d.node('ll', cx(1), cy(1), cw, RH, 'process', { name: 'llama-server', tech: '«executable» prebuilt llama.cpp', desc: 'Downloaded runtime: the GGUF model and its vision projector.' });
  d.node('ch', cx(1), cy(2), cw, RH, 'process', { name: 'Chromium', tech: '«executable» Edge · Chrome · Chromium', desc: 'Driven over DevTools for research; off screen unless shown.' });
  d.note('sb', cx(1), cy(3), cw, 'Sandbox: Windows AppContainer or low integrity with a job object; Linux landlock and seccomp; macOS sandbox profile.', RH);
  link(d, 'ui', 'B', 'app', 'T', { label: 'Tauri IPC', t: [0.9, 0.9] });
  link(d, 'app', 'B', 'data', 'T', { label: 'Reads, writes' });
  const kr = (y) => (y - ky) / kh;
  d.edge('ap', 'kids', [72 + 24 + NW + 24, Math.round(ay + 40 + (RH + 24) / 2)], [kx, Math.round(ay + 40 + (RH + 24) / 2)], {});
  d.lbl('Spawns, calls', (72 + 24 + NW + 24 + kx) / 2, ay + 40 + (RH + 24) / 2, 'a');
  link(d, 'aw', 'B', 'll', 'T', { label: 'Starts' });
  d.legend([{ kind: 'accent', label: 'Debrief executable' }, { kind: 'ui', label: 'UI' }, { kind: 'plugin', label: 'Plugin' }, { kind: 'process', label: 'Process · data' }, { kind: 'line', label: 'Communication' }]);
  save('uml-deployment-workstation', d, my + mh);
}

// ---------- monitored server ----------
{
  const d = P({ title: 'Debrief — deployment on a monitored server', kind: 'UML · Deployment diagram (2 of 3)', w: 1280,
    desc: 'Collection runs headless on the server; what it writes is copied to the analyst workstation and opened there.' });
  const NW = 264, RH = 112, mx = 48, my = Y0;
  const mh = 48 + 40 + 3 * RH + 2 * 24 + 24 + 24;
  const outX = 72 + (NW + 112) + 24;
  d.group('m', mx, my, outX + NW + 24 - mx, mh, '«device» Monitored server [Windows Server · Linux · macOS]');
  const x = (i) => 72 + i * (NW + 112), ry = (i) => my + 48 + 40 + i * (RH + 24);
  d.group('sv', x(0) - 24, my + 48, 24 + NW + 24, 40 + 3 * RH + 2 * 24 + 24, '«execution environment» Debrief tools');
  d.node('ag', x(0), ry(0), NW, RH, 'accent', { name: 'imt-agent', tech: '«service» SCM · systemd · launchd', desc: 'Scheduled and triggered collection; retention by count, age and size.' });
  d.node('co', x(0), ry(1), NW, RH, 'accent', { name: 'imt-collect', tech: '«executable» CLI', desc: 'A redacted diagnostics bundle.' });
  d.node('mo', x(0), ry(2), NW, RH, 'accent', { name: 'imt-monitor', tech: '«executable» CLI', desc: 'Process and counter sampling into rolling CLEF.' });
  d.node('os', x(1) + 24, ry(0), NW, RH, 'external', { name: 'OS sources', tech: 'Event Log · PDH · `logman` · logs', desc: 'What the agent reads and runs, with timeouts.' });
  d.node('out', x(1) + 24, ry(1), NW, 2 * RH + 24, 'process', { name: 'Output folder', tech: '«folder» run folders · `.imtz` · rolling CLEF', desc: 'A versioned `manifest.json` per run.' });
  const wx = outX + NW + 24 + 128;
  d.node('ws', wx, ry(1) + (RH + 24) / 2, NW, RH, 'accent', { name: 'Analyst workstation', tech: '«device» see the workstation diagram', desc: 'Opens the bundles and the CLEF files with Debrief or `imt`.' });
  link(d, 'ag', 'R', 'os', 'L', { label: 'Reads, runs' });
  link(d, 'co', 'R', 'out', 'L', { label: 'Writes' });
  link(d, 'mo', 'R', 'out', 'L', { label: 'Writes' });
  link(d, 'out', 'R', 'ws', 'L', { label: 'Copied' });
  d.legend([{ kind: 'accent', label: 'Debrief executable' }, { kind: 'process', label: 'Folder' }, { kind: 'external', label: 'External source' }, { kind: 'line', label: 'Communication' }]);
  save('uml-deployment-server', d, my + mh);
}

// ---------- build and release ----------
{
  const d = P({ title: 'Debrief — deployment of the build and the downloads', kind: 'UML · Deployment diagram (3 of 3)', w: 1056,
    desc: 'Gitea Actions builds on three kinds of runner and publishes the installers. The app reaches the network only for updates, models, runtimes and research.' });
  const NW = 296, G = 168, x0 = 48, x1 = x0 + NW + 48 + G;
  const N = (id, role, o) => [id, role, o];
  const g1 = grp(d, 'gt', x0, Y0, '«device» Gitea server [dev.decryptology.net]', [
    N('gi', 'external', { name: 'Gitea', tech: '«application» git, Actions, releases', desc: 'Workflows `build`, `fuzz`, `bench`, `mutants`, `corpus` and `runner-check`; releases on `v*` tags.' }),
    N('ra', 'process', { name: 'Release assets', tech: '«artifact» installers and updater manifest', desc: 'NSIS `.exe`, `.dmg`, `.deb`, `.rpm` and the signed `latest.json`.' }),
  ], { nw: NW, minH: 392 });
  grp(d, 'rn', x1, Y0, '«execution environment» act_runner hosts', [
    N('u', 'process', { name: 'ubuntu-latest', tech: '«node»', desc: 'Lint, tests, front end, `.deb` and `.rpm`, smoke test, the release job.' }),
    N('w', 'process', { name: 'windows-latest', tech: '«node»', desc: 'Smoke test of the real app and the NSIS `.exe`.' }),
    N('m', 'process', { name: 'macos-latest', tech: '«node»', desc: 'The `.dmg` and the app archive.' }),
  ], { nw: NW, minH: 392 });
  const y2 = Y0 + 392 + 72;
  d.node('wk', x0 + 24, y2, NW, null, 'accent', { name: 'Analyst workstation', tech: '«device» the app', desc: 'Updates, models, runtimes and research: only when the user starts them.' });
  grp(d, 'hosts', x1, y2, '«cloud» Download hosts [HTTPS only]', [
    N('hf', 'external', { name: 'Hugging Face', tech: 'huggingface.co', desc: 'GGUF models, opt-in.' }),
    N('gh', 'external', { name: 'GitHub releases', tech: 'llama.cpp · FFmpeg builds', desc: 'Pinned `llama-server` and LGPL FFmpeg, SHA-256 checked.' }),
    N('cf', 'external', { name: 'Chrome for Testing', tech: 'optional Chromium', desc: 'Pinned; only on request.' }),
  ], { nw: NW });
  link(d, 'gi', 'R', 'rn', 'L', { label: 'Dispatches' });
  link(d, 'rn', 'L', 'ra', 'R', { label: 'Uploads' });
  link(d, 'wk', 'T', 'ra', 'B', { label: 'Checks' });
  link(d, 'wk', 'R', 'hosts', 'L', { label: 'Downloads' });
  d.legend([{ kind: 'accent', label: 'Debrief' }, { kind: 'process', label: 'Node · artefact' }, { kind: 'external', label: 'External system' }, { kind: 'line', label: 'Communication' }]);
  save('uml-deployment-release', d, Math.max(d.rect.hosts[1] + d.rect.hosts[3], d.rect.wk[1] + d.rect.wk[3]));
}
