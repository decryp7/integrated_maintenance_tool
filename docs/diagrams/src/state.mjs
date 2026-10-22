import { P, save, link, route, selfLoop, dot, grp, bottom, Y0 } from './common.mjs';

const S = (d, id, x, y, w, h, name, desc, role = 'accent') => d.node(id, x, y, w, h, role, { name, desc });

// ---------- live tail 1 ----------
{
  const d = P({ title: 'Live tail — the worker per window', kind: 'UML · State machine (1 of 2)', w: 1344,
    desc: '`set_live` drives one live-tail worker per window. The store reads what was appended and the window’s `LiveSession` evaluates alerts over the ports.' });
  const WS = 200, WH = 120, GAP = 120, gx = 400, gy = Y0;
  const gh = 64 + WH + 56, gw = 24 + 3 * WS + 2 * GAP + 24;
  const oy = gy + 64, oc = oy + WH / 2;
  S(d, 'off', 72, oy, 200, WH, 'Off', 'Nothing runs. The tray and the Live toggle are idle.', 'process');
  dot(d, 'ini', 72 + 100 - 8, Y0 - 8 + 0, 'start');
  d.group('live', gx, gy, gw, gh, 'Live [thread `imt-live-tail`]');
  const sx = (i) => gx + 24 + i * (WS + GAP);
  S(d, 'wat', sx(0), oy, WS, WH, 'Watching', 'Notify events per root, or a poll every 2 s on network paths; a tick every 1 s.');
  S(d, 'coa', sx(1), oy, WS, WH, 'Coalescing', 'Changed paths are batched; more than 100,000 pending means a full rescan.');
  S(d, 'ref', sx(2), oy, WS, WH, 'Refreshing', '`Ingest::append_tail`: only what was appended; the index follows; the last event waits 500 ms.');
  const fy = gy + gh + 56;
  dot(d, 'fin', 72 + 100 - 12, fy, 'end');
  link(d, 'ini', 'B', 'off', 'T');
  const fr = (y) => (y - gy) / gh;
  route(d, 'off', 'R', 0.3, 'live', 'L', fr(oy + WH * 0.3));
  d.lbl('set_live(true)', (272 + gx) / 2, oy + WH * 0.3, 'a');
  route(d, 'live', 'L', fr(oy + WH * 0.75), 'off', 'R', 0.75);
  d.lbl('set_live(false)', (272 + gx) / 2, oy + WH * 0.75, 'a');
  link(d, 'wat', 'R', 'coa', 'L', { label: 'paths changed' });
  link(d, 'coa', 'R', 'ref', 'L', { label: 'settled' });
  const wb = d.port('wat', 'B', 0.5), rb = d.port('ref', 'B', 0.5), yb = oy + WH + 28;
  route(d, 'wat', 'B', 0.5, 'ref', 'B', 0.5, [[wb[0], yb], [rb[0], yb]]);
  d.lbl('tick', (wb[0] + rb[0]) / 2, yb, 'b');
  const wt = d.port('wat', 'T', 0.5), rt = d.port('ref', 'T', 0.5), yt = gy + 44;
  route(d, 'ref', 'T', 0.5, 'wat', 'T', 0.5, [[rt[0], yt], [wt[0], yt]]);
  d.lbl('report emitted', (wt[0] + rt[0]) / 2, yt, 'a');
  const ex = gx + 56;
  route(d, 'live', 'B', 56 / gw, 'fin', 'R', 0.5, [[ex, fy + 12]]);
  d.lbl('window closed', (ex + 72 + 100 + 12) / 2 + 12, fy + 12, 'a');
  route(d, 'off', 'B', 0.5, 'fin', 'T', 0.5);
  d.lbl('window closed', 72 + 100, (oy + WH + fy) / 2, 'r');
  d.note('n1', 640, fy - 8, 328, 'The tray’s stop emits `live-stopped`. `set_live(true)` while running restarts with the current roots. A report that is not empty emits `workspace-changed`, and `live-alert` for new alerts.');
  d.note('n2', 1000, fy - 8, 296, 'Timing: `liveTailDebounce` (default 750 ms) sets the quiet time, the longest delay and the shortest gap between refreshes.');
  d.legend([{ kind: 'process', label: 'Idle' }, { kind: 'accent', label: 'State' }, { kind: 'line', label: 'Event / action' }]);
  save('uml-state-live-tail', d, Math.max(d.rect.n1[1] + d.rect.n1[3], d.rect.n2[1] + d.rect.n2[3]));
}

// ---------- live tail 2: alerts and follow ----------
{
  const d = P({ title: 'Live tail — alerts and the timeline', kind: 'UML · State machine (2 of 2)', w: 1344,
    desc: 'One alert machine per detector and instance (or per `[[live.rule]]`), and the UI’s follow mode on the timeline. `LiveSession` evaluates alerts over the store ports (ADR 0037).' });
  const W = 208, H = 88, GAP = 160;
  const gx = 48, gy = Y0, gh = 64 + H + 56, gw = 24 + 3 * W + 2 * GAP + 24;
  d.group('al', gx, gy, gw, gh, 'Alert [per detector kind and instance]');
  const sy = gy + 64, sx = (i) => gx + 24 + i * (W + GAP);
  S(d, 'cool', sx(0), sy, W, H, 'Cooling down', 'No new alert until the cooldown has passed (default 5 minutes).');
  S(d, 'arm', sx(1), sy, W, H, 'Armed', 'Watching the events; ready to raise.');
  S(d, 'mut', sx(2), sy, W, H, 'Muted', 'Nothing is raised and no cooldown starts, for up to 24 hours.', 'process');
  dot(d, 'ini', sx(1) + W / 2 - 8, gy + 36 - 4, 'start');
  link(d, 'ini', 'B', 'arm', 'T');
  route(d, 'arm', 'L', 0.3, 'cool', 'R', 0.3);
  d.lbl('detector fires', (sx(0) + W + sx(1)) / 2, sy + H * 0.3, 'a');
  route(d, 'cool', 'R', 0.75, 'arm', 'L', 0.75);
  d.lbl('cooldown elapsed', (sx(0) + W + sx(1)) / 2, sy + H * 0.75, 'a');
  route(d, 'arm', 'R', 0.3, 'mut', 'L', 0.3);
  d.lbl('mute_alerts', (sx(1) + W + sx(2)) / 2, sy + H * 0.3, 'a');
  route(d, 'mut', 'L', 0.75, 'arm', 'R', 0.75);
  d.lbl('unmute or expiry', (sx(1) + W + sx(2)) / 2, sy + H * 0.75, 'a');
  selfLoop(d, 'cool', 'fires: suppressed', { side: 'B', t: [0.25, 0.75] });
  selfLoop(d, 'mut', 'fires: not raised', { side: 'B', t: [0.25, 0.75] });
  const fy = gy + gh + 72, fw = 24 + 2 * 208 + 160 + 24;
  d.group('fo', 48, fy, fw, 64 + H + 24, 'Timeline follow [UI · `LiveFollowStore`]');
  S(d, 'fol', 48 + 24, fy + 64, 208, H, 'Following', 'The timeline keeps the newest events in view.', 'ui');
  S(d, 'hol', 48 + 24 + 208 + 160, fy + 64, 208, H, 'Holding', 'The view stays where the user put it while events arrive.', 'ui');
  route(d, 'fol', 'R', 0.3, 'hol', 'L', 0.3);
  d.lbl('zoom, pan, pause', 48 + 24 + 208 + 80, fy + 64 + H * 0.3, 'a');
  route(d, 'hol', 'L', 0.75, 'fol', 'R', 0.75);
  d.lbl('resume', 48 + 24 + 208 + 80, fy + 64 + H * 0.75, 'a');
  d.note('n1', 48 + fw + 96, fy, 1296 - (48 + fw + 96), 'Alerts: the profile’s `[live]` section names the detectors (error spike, silence, crash, slow outlier, rate change, memory growth) and rules; silence after 60 s, cooldown 5 minutes. Desktop notices only while the window is not seen: at most one a kind every 30 s and five every 5 minutes.');
  d.legend([{ kind: 'accent', label: 'State' }, { kind: 'process', label: 'Muted' }, { kind: 'ui', label: 'UI state' }, { kind: 'line', label: 'Event / action' }]);
  save('uml-state-live-alerts', d, fy + 64 + H + 24);
}

// ---------- perfmon ----------
{
  const d = P({ title: 'Performance Monitor — a collector set', kind: 'UML · State machine', w: 1344,
    desc: 'A Data Collector Set as Debrief drives it: `PerfmonController` in the tools plugin runs `logman`; nothing touches the registry.' });
  const W = 224, H = 112, gx = 520, gy = Y0, gw = 24 + 2 * W + 168 + 24, gh = 64 + H + 40;
  const rowY = gy + 64;
  S(d, 'not', 104, rowY, W, H, 'Not installed', 'No set with this name.', 'process');
  dot(d, 'ini', 48, rowY + H / 2 - 8, 'start');
  d.group('ins', gx, gy, gw, gh, 'Installed');
  S(d, 'stp', gx + 24, rowY, W, H, 'Stopped', 'The set exists and is not collecting.');
  S(d, 'run', gx + 24 + W + 168, rowY, W, H, 'Running', 'Writing counter logs (`.blg`, `.csv`, `.tsv`); the task `perfmon:<name>` is registered.');
  const ux = gx + gw / 2 - W / 2;
  S(d, 'unk', ux, gy + gh + 96, W, H - 24, 'Unknown', 'The status text was not parsed.', 'process');
  link(d, 'ini', 'R', 'not', 'L', {});
  const fr = (y) => (y - gy) / gh;
  route(d, 'not', 'R', 0.3, 'ins', 'L', fr(rowY + H * 0.3));
  d.lbl('install', (104 + W + gx) / 2, rowY + H * 0.3, 'a');
  route(d, 'ins', 'L', fr(rowY + H * 0.75), 'not', 'R', 0.75);
  d.lbl('delete', (104 + W + gx) / 2, rowY + H * 0.75, 'a');
  route(d, 'stp', 'R', 0.3, 'run', 'L', 0.3);
  d.lbl('start', gx + 24 + W + 84, rowY + H * 0.3, 'a');
  route(d, 'run', 'L', 0.75, 'stp', 'R', 0.75);
  d.lbl('stop', gx + 24 + W + 84, rowY + H * 0.75, 'a');
  selfLoop(d, 'stp', 'install: replace', { side: 'T', t: [0.3, 0.7] });
  const x1 = ux + 56, x2 = ux + W - 56;
  route(d, 'ins', 'B', (x1 - gx) / gw, 'unk', 'T', 56 / W);
  d.lbl('query: not parsed', x1, gy + gh + 48, 'l');
  route(d, 'unk', 'T', (W - 56) / W, 'ins', 'B', (x2 - gx) / gw);
  d.lbl('query: parsed', x2, gy + gh + 48, 'r');
  d.note('n1', 104, rowY + H + 72, 328, 'A failed `logman` call leaves the state unchanged; the view shows the error: unsupported (not Windows), access denied (run elevated), invalid set name, or `logman` did not start or timed out (120 s).');
  d.note('n2', 104, d.rect.n1[1] + d.rect.n1[3] + 24, 328, '`outputs(name)` lists `.blg`, `.csv` and `.tsv` files under the root and output paths (depth at most 4), newest first; Import results opens them in Debrief.');
  d.legend([{ kind: 'accent', label: 'State' }, { kind: 'process', label: 'Absent or unknown' }, { kind: 'line', label: 'Event / action' }]);
  save('uml-state-perfmon', d, Math.max(d.rect.n2[1] + d.rect.n2[3], d.rect.unk[1] + d.rect.unk[3]));
}

// ---------- project ----------
{
  const d = P({ title: 'Project — saved, unsaved and modified', kind: 'UML · State machine (1 of 2)', w: 1344,
    desc: 'The open project as `ProjectService` tracks it. A project is the window’s store folder saved as `.imtz`; it opens without the original logs.' });
  const W = 192, H = 88, SG = 112, gw = 24 + 2 * W + SG + 24, gh = 64 + H + 24;
  const g1 = 48, g2 = 48 + gw + 232, gy = Y0 + 40;
  d.group('un', g1, gy, gw, gh, 'Untitled [path = null]');
  d.group('sv', g2, gy, gw, gh, 'Saved [path set]');
  const sy = gy + 64;
  S(d, 'uc', g1 + 24, sy, W, H, 'Clean', 'The snapshot equals the baseline.');
  S(d, 'ud', g1 + 24 + W + SG, sy, W, H, 'Dirty', 'Sources, UI state or reports differ from the baseline.');
  S(d, 'sc', g2 + 24, sy, W, H, 'Clean', 'Equals what was last written.');
  S(d, 'sm', g2 + 24 + W + SG, sy, W, H, 'Modified', 'Differs from what was last written.');
  dot(d, 'ini', g1 + 24 + W * 0.85 - 8, Y0 - 4, 'start');
  route(d, 'ini', 'B', 0.5, 'uc', 'T', 0.85);
  for (const [a, b, l1, l2, x0] of [['uc', 'ud', 'edited', 'undone', g1 + 24], ['sc', 'sm', 'edited', 'reverted, saved', g2 + 24]]) {
    route(d, a, 'R', 0.3, b, 'L', 0.3); d.lbl(l1, x0 + W + SG / 2, sy + H * 0.3, 'a');
    route(d, b, 'L', 0.75, a, 'R', 0.75); d.lbl(l2, x0 + W + SG / 2, sy + H * 0.75, 'a');
  }
  const fr = (y) => (y - gy) / gh;
  route(d, 'un', 'R', fr(sy + H * 0.3), 'sv', 'L', fr(sy + H * 0.3));
  d.lbl('save, open', (g1 + gw + g2) / 2, sy + H * 0.3, 'a');
  route(d, 'sv', 'L', fr(sy + H * 0.75), 'un', 'R', fr(sy + H * 0.75));
  d.lbl('new, clear', (g1 + gw + g2) / 2, sy + H * 0.75, 'a');
  const ny = gy + gh + 72;
  d.note('n1', 48, ny, 592, 'Unsaved changes: New, Open and Open recent ask Save, Discard or Cancel while the project is dirty (Escape cancels); Save continues only if it succeeds. Closing a window does not ask. Dirty means the sources, the tracked UI snapshot or the reports differ from the baseline.');
  d.note('n2', 688, ny, 592, 'Save writes `save_project` as `.imtz` and sets the baseline to what was written; Save as goes through the picker and the overwrite guard. Opening fails with a message (cannot read, not a Debrief project, newer format, store locked) and leaves the state unchanged.');
  d.legend([{ kind: 'accent', label: 'State' }, { kind: 'line', label: 'Event / action' }]);
  save('uml-state-project', d, Math.max(d.rect.n1[1] + d.rect.n1[3], d.rect.n2[1] + d.rect.n2[3]));
}

// ---------- session ----------
{
  const d = P({ title: 'Session — start, recovery and autosave', kind: 'UML · State machine (2 of 2)', w: 1344,
    desc: 'One app run: a marker file detects a crash, recovery is offered, and the session is autosaved while the window is open.' });
  const W = 232, H = 104;
  const sy = Y0 + 64, ry = sy + H + 128;
  dot(d, 'ini', 72 + W / 2 - 8, Y0 + 8, 'start');
  S(d, 'sta', 72, sy, W, H, 'Starting', 'Read `running.marker`, write its own, rotate the last sessions into `recovered/`.');
  S(d, 'rec', 72, ry, W, H, 'Recovery offer', 'A banner offers the last session and autosave pauses until the user chooses.');
  dot(d, 'sm', 72 + W / 2 - 12, ry + H + 80, 'end');
  const gx = 520, gw = 24 + 264 + 24, gh = ry + H + 24 - Y0;
  d.group('run', gx, Y0, gw, gh, 'Running');
  S(d, 'idl', gx + 24, sy, 264, H, 'Idle', 'The session on disk is current.');
  S(d, 'pen', gx + 24, ry, 264, H, 'Save pending', 'A 10 s timer is armed.');
  dot(d, 'ex', gx + gw + 168, sy + H / 2 - 12, 'end');
  link(d, 'ini', 'B', 'sta', 'T', {});
  const fr = (y) => (y - Y0) / gh;
  route(d, 'sta', 'R', 0.5, 'run', 'L', fr(sy + H / 2));
  d.lbl('no marker', (72 + W + gx) / 2, sy + H / 2, 'a');
  link(d, 'sta', 'B', 'rec', 'T', { label: 'dead process' });
  route(d, 'rec', 'R', 0.5, 'run', 'L', fr(ry + H / 2));
  d.lbl('restore, dismiss', (72 + W + gx) / 2, ry + H / 2, 'a');
  link(d, 'rec', 'B', 'sm', 'T', { label: 'safe mode' });
  route(d, 'idl', 'B', 0.3, 'pen', 'T', 0.3);
  d.lbl('snapshot changed', gx + 24 + 264 * 0.3, (sy + H + ry) / 2, 'l');
  route(d, 'pen', 'T', 0.7, 'idl', 'B', 0.7);
  d.lbl('quiet 10 s, or blur', gx + 24 + 264 * 0.7, (sy + H + ry) / 2, 'r');
  selfLoop(d, 'pen', 'changed: restart', { side: 'B', t: [0.25, 0.75] });
  route(d, 'run', 'R', fr(sy + H / 2), 'ex', 'L', 0.5);
  d.lbl('close, exit', (gx + gw + gx + gw + 168) / 2 - 8, sy + H / 2, 'a');
  d.note('n1', gx + gw + 96 - 0, ry - 8, 1296 - (gx + gw + 96), 'On “snapshot changed” the 10 s timer starts; after 10 quiet seconds or when the window loses focus the session is written with `save_session`. Closing a window forgets its session; exit removes the marker. Safe mode restarts without plugins.');
  d.note('n2', gx + gw + 96, d.rect.n1[1] + d.rect.n1[3] + 24, 1296 - (gx + gw + 96), 'Sessions live in `sessions/<window>/last-session.imtproj` with `manifest.json` (version 2); a missing or corrupt manifest falls back to scanning the folders.');
  d.legend([{ kind: 'accent', label: 'State' }, { kind: 'line', label: 'Event / action' }]);
  save('uml-state-session', d, Math.max(d.rect.n2[1] + d.rect.n2[3], gh + Y0, sm_bottom(d)));
  function sm_bottom(dd) { return dd.rect.sm[1] + dd.rect.sm[3]; }
}

// ---------- watched file ----------
{
  const d = P({ title: 'Watched folders — one file', kind: 'UML · State machine', w: 1344,
    desc: 'What `WatchState` decides for each file a polling scan sees (ADR 0048). A pure state machine with a fake clock in its tests.' });
  const W = 208, H = 88, G = 120, x = (i) => 72 + i * (W + G), r1 = Y0 + 64, r2 = r1 + H + 96;
  dot(d, 'ini', x(0) + W / 2 - 8, Y0 + 4, 'start');
  S(d, 'set', x(0), r1, W, H, 'Settling', 'Size and time must stay the same for 2 s. Temporary names and empty files are ignored.');
  S(d, 'rdy', x(1), r1, W, H, 'Ready', 'Quiet long enough, and any retry time has passed.');
  S(d, 'imp', x(2), r1, W, H, 'Importing', 'An import job: `open` for a new file, `refresh` for a known one.');
  S(d, 'don', x(3), r1, W, H, 'Imported', 'Remembered with its size and time.', 'core');
  S(d, 'big', x(0), r2, W, H, 'Too large', 'Over 4 GiB: reported once and left to the user.', 'process');
  S(d, 'bak', x(2), r2, W, H, 'Backing off', 'Locked or unreadable: tried again after 2 s, 4 s, 8 s.');
  S(d, 'giv', x(3), r2, W, H, 'Given up', 'Reported once. A file that changes is tried again.', 'process');
  link(d, 'ini', 'B', 'set', 'T', {});
  link(d, 'set', 'R', 'rdy', 'L', { label: 'quiet 2 s' });
  link(d, 'rdy', 'R', 'imp', 'L', { label: 'job starts' });
  route(d, 'imp', 'R', 0.5, 'don', 'L', 0.5);
  d.lbl('imported', (x(2) + W + x(3)) / 2, r1 + H / 2, 'a');
  link(d, 'imp', 'B', 'bak', 'T', { label: 'fails' });
  const gapx = x(2) + W + G / 2;
  route(d, 'imp', 'R', 0.85, 'giv', 'L', 0.5, [[gapx, r1 + H * 0.85], [gapx, r2 + H / 2]]);
  d.lbl('fourth failure', gapx + 6, r1 + H + 24, 'r');
  route(d, 'bak', 'L', 0.5, 'rdy', 'B', 0.5, [[x(1) + W / 2, r2 + H / 2]]);
  d.lbl('delay over', x(1) + W + 0, r2 + H / 2, 'a');
  link(d, 'set', 'B', 'big', 'T', { label: 'over 4 GiB' });
  const dt = d.port('don', 'T', 0.5), st = d.port('set', 'T', 0.5), ty = Y0 + 36;
  route(d, 'don', 'T', 0.5, 'set', 'T', 0.7, [[dt[0], ty], [d.port('set', 'T', 0.7)[0], ty]]);
  d.lbl('file changed', (dt[0] + d.port('set', 'T', 0.7)[0]) / 2, ty, 'a');
  selfLoop(d, 'set', 'changed: restart', { side: 'T', t: [0.1, 0.3] });
  d.note('n1', x(0), r2 + H + 56, 4 * W + 3 * G, 'A file that vanishes is forgotten. At most 10,000 files are pending and a scan reads at most 50,000 entries, 16 levels deep, without following symbolic links. A new known issue found in the imported data is announced once, or again when it has more events.');
  d.legend([{ kind: 'accent', label: 'State' }, { kind: 'core', label: 'Done' }, { kind: 'process', label: 'Left alone' }, { kind: 'line', label: 'Event / action' }]);
  save('uml-state-watched-file', d, d.rect.n1[1] + d.rect.n1[3]);
}
