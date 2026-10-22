import { P, save, link, hub, dot, bottom, Y0, comp } from './common.mjs';

const ACT = (d, id, x, y, w, name, desc, role = 'accent', tech) => d.node(id, x, y, w, null, role, { name, tech, desc });

// ---------- ingest 1: a file to records ----------
{
  const d = P({ title: 'Ingest — from a file to records', kind: 'UML · Activity diagram (1 of 2)', w: 1280,
    desc: 'In the store worker, one file at a time, under `guard::catch`. Every failure lists the file as unrecognised with its reason; the rest of the import continues.' });
  const MW = 400, mx = 96, sx = mx + MW + 24 + 168;
  dot(d, 'st', mx + MW / 2 - 8, Y0 + 8, 'start');
  const acts = [
    ['rd', 'Read the bytes', '`read_with_retry`: four retries while the file is locked.', [['I/O error or locked']]],
    ['ex', 'Expand an archive', '`archive::expand` for zip and gzip: at most 1 GiB an entry and 4 GiB in all.', [['Invalid or too big']]],
    ['pl', 'Plan from the first 64 KiB', 'The parser library picks a parser, then the importer and the text encoding (BOM, detection or the source’s own choice).', [['Empty file'], ['Binary file'], ['No importer']]],
    ['im', 'Read the events', 'The importer reads 8 MiB chunks of whole lines; lines over 4 MiB are skipped. A binary importer reads the bytes.', [['Importer error'], ['No events']]],
  ];
  let y = Y0 + 56;
  const ys = {};
  // region frame first (draw.io z-order): compute heights
  const hs = acts.map(([, n, ds]) => d.fit(MW, { name: n, desc: ds }));
  const regionTop = y + 0 + hs[0] + 40;
  const total = hs.reduce((a, b) => a + b, 0) + 40 * (acts.length - 1);
  const regionH = hs[1] + hs[2] + hs[3] + 80 + 40 + 24;
  d.group('reg', 72, regionTop - 40, MW + 48, regionH, '«iterative»  each entry');
  acts.forEach(([id, n, ds], i) => { d.node(id, mx, y, MW, hs[i], 'accent', { name: n, desc: ds }); ys[id] = y; y += hs[i] + 40; });
  y += 32;
  dot(d, 'en', mx + MW / 2 - 12, y + 8, 'end');
  // unrecognised sink
  const sy = Y0 + 56, sh = ys.im + hs[3] + 24 - sy;
  d.node('sink', sx, sy, 296, sh, 'process', { name: 'List as unrecognised', tech: '`UnrecognizedFile`: path, name, reason', desc: 'Shown in the import summary with its reason. The other files still import. A panic becomes “crashed (please report this file)”.' });
  link(d, 'st', 'B', 'rd', 'T');
  link(d, 'rd', 'B', 'ex', 'T');
  link(d, 'ex', 'B', 'pl', 'T');
  link(d, 'pl', 'B', 'im', 'T');
  link(d, 'im', 'B', 'en', 'T', { label: 'To ingest' });
  for (const [id, , , exits] of acts) {
    const [x0, y0, w0, h0] = d.rect[id];
    exits.forEach(([lab], k) => {
      const yy = Math.round(y0 + h0 * ((k + 1) / (exits.length + 1)));
      d.edge(id, 'sink', [x0 + w0, yy], [sx, yy], {});
      d.lbl(lab, (x0 + w0 + sx) / 2, yy, 'a');
    });
  }
  d.legend([{ kind: 'accent', label: 'Action' }, { kind: 'process', label: 'Unrecognised file' }, { kind: 'line', label: 'Control flow' }]);
  save('uml-activity-ingest-read', d, y + 40);
}

// ---------- ingest 2: records to events ----------
{
  const d = P({ title: 'Ingest — from records to events', kind: 'UML · Activity diagram (2 of 2)', w: 1248,
    desc: 'What `RecordBatcher` and `CompiledProfile::ingest` do to each batch of at most 8,192 records before the store writes it.' });
  const MW = 440, mx = 72;
  const acts = [
    ['wi', 'Drop events outside the import window', '`RecordBatcher::with_window` (ADR 0047), only when a window is set. An event with no time is outside any window. Dropped events are counted.'],
    ['ex', 'Run the extractors', '`apply_extractors`: every `[[extract]]` whose `when` holds adds fields; the parser’s own extractors run first.'],
    ['pi', 'Run the pipeline', '`Pipeline::run_marking`: the parser’s steps, then `[[pipeline]]` in profile order. A step runs on the records its `when` selects and keeps the rest unchanged.'],
    ['mk', 'Compute the markers', '`compute_markers`: each marker kind whose `when` holds sets a bit in the `MarkerMask`; plugin transforms may have marked the record already.'],
    ['wr', 'Write the batch', 'The store writer assigns ids, applies the source’s clock offset, writes Parquet segments, adds the `SourceInfo` to the catalog and indexes the words.'],
  ];
  dot(d, 'st', mx + MW / 2 - 8, Y0 + 8, 'start');
  let y = Y0 + 56;
  acts.forEach(([id, n, ds]) => { d.node(id, mx, y, MW, null, 'accent', { name: n, desc: ds }); y += d.rect[id][3] + 40; });
  dot(d, 'en', mx + MW / 2 - 12, y - 8, 'end');
  link(d, 'st', 'B', 'wi', 'T');
  for (let i = 0; i < acts.length - 1; i++) link(d, acts[i][0], 'B', acts[i + 1][0], 'T');
  link(d, 'wr', 'B', 'en', 'T');
  const nx = mx + MW + 96, nw = 1248 - nx - 48;
  const ny = d.rect.pi[1];
  d.node('s1', nx, d.rect.pi[1], nw, null, 'process', { name: 'Step kinds', desc: 'Transform: drop, keep, mask, remove_fields, rename, set, set_level, truncate, parse_json, parse_kv, lookup, shift_time, calc, or a plugin step by installed id. Dedupe: collapses repeats of timed records in a window and adds RepeatCount. Script: Rhai, sandboxed, at most 100,000 operations.' });
  d.node('s2', nx, d.rect.s1[1] + d.rect.s1[3] + 40, nw, null, 'process', { name: 'Failure rules', desc: 'A calc or script that does not compile is a profile error, so it never reaches ingest. At run time a calc that cannot evaluate leaves the record unchanged; a script error keeps the record and counts it in StepStats. Disabled steps stay as placeholders so step indexes line up.' });
  link(d, 'pi', 'R', 's1', 'L', { dash: true, arrow: 'none' });
  d.legend([{ kind: 'accent', label: 'Action' }, { kind: 'process', label: 'Note' }, { kind: 'line', label: 'Control flow' }]);
  save('uml-activity-ingest-events', d, Math.max(y + 24, d.rect.s2[1] + d.rect.s2[3]));
}

// ---------- video ----------
{
  const d = P({ title: 'Video sources — from a recording to events', kind: 'UML · Activity diagram', w: 1472,
    desc: 'Sandboxed FFmpeg passes, a pure frame selection, alignment to the logs and the vision model (ADR 0033). Each frame becomes one event.' });
  const W = 288, G = 96, x = (i) => 48 + i * (W + G), H = 136, rowY = (r) => Y0 + r * (H + 56);
  const A = (id, col, row, role, name, tech, desc) => d.node(id, x(col), rowY(row), W, H, role, { name, tech, desc });
  A('a1', 0, 0, 'accent', 'Open a video', 'File ▸ Open or drop', 'Sniffed by signature; the player gets the file through a `media:` scheme.');
  A('a2', 1, 0, 'accent', 'Check the runtime', 'Settings ▸ Video', 'Pinned LGPL FFmpeg or the user’s own; if missing, a job installs it: download, SHA-256, unpack, self-test, rename into place.');
  A('a3', 2, 0, 'accent', 'Probe', '`ffprobe`, sandboxed', 'Duration, size and container time. Limits: 6 hours, 30 s probe timeout.');
  A('a4', 3, 0, 'accent', 'Pass 1: candidates', '`mpdecimate` + scene score', 'Scaled to 256 px; drops blinking cursors and clocks; scores every remaining frame.');
  A('a5', 3, 1, 'core', 'Select frames', '`imt_video::select`, pure', 'First and last always; the threshold rises until at most 30 (120 at most); or one per interval.');
  A('a6', 2, 1, 'accent', 'Pass 2: extract', 'PNG, at most 1,366 px', 'Decodes exactly the selected times; each PNG is re-encoded in Rust.');
  A('a7', 1, 1, 'ui', 'Frame strip', 'Video view', 'Drop frames, add the current one, switch mode and analyse again.');
  A('a8', 0, 1, 'core', 'Align to the logs', '`imt_video::align`, pure', 'User start, on-screen clock, file name, container time or file time, each with a confidence.');
  A('a9', 0, 2, 'accent', 'Describe frames', 'AI job, vision model', 'One request per kept frame; screen text is data, never instructions.');
  A('a10', 1, 2, 'accent', 'Write `.frames`', '`videos/` in the project', 'Versioned `video.json`, the frames and the frames source; the video is referenced by SHA-256.');
  A('a11', 2, 2, 'core', 'VideoFramesImporter', 'One event per frame', 'Its own lane (Video): timeline, search, Moment, notes and SQL work unchanged.');
  d.note('nt', x(3), rowY(2), W, 'Every FFmpeg run: empty environment, no network, reads only the runtime folder and the chosen video, writes only a fresh temporary folder, memory-capped, time-limited and cancellable from the Jobs pane. A model without vision is never sent images.', H);
  for (const [a, b] of [['a1', 'a2'], ['a2', 'a3'], ['a3', 'a4']]) link(d, a, 'R', b, 'L');
  link(d, 'a4', 'B', 'a5', 'T');
  for (const [a, b] of [['a5', 'a6'], ['a6', 'a7'], ['a7', 'a8']]) link(d, a, 'L', b, 'R');
  link(d, 'a8', 'B', 'a9', 'T');
  for (const [a, b] of [['a9', 'a10'], ['a10', 'a11']]) link(d, a, 'R', b, 'L');
  d.legend([{ kind: 'accent', label: 'Engine or job' }, { kind: 'core', label: 'Pure function' }, { kind: 'ui', label: 'UI' }, { kind: 'line', label: 'Control flow' }]);
  save('uml-activity-video', d, rowY(2) + H);
}
