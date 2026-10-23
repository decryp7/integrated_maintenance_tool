import { P, save, link, realize, klass, comp, bottom, Y0 } from './common.mjs';
const LEG = (extra = []) => [{ kind: 'accent', label: 'Type' }, { kind: 'core', label: 'Enumeration' }, { kind: 'line', label: 'Association (diamond: composition)' }, { kind: 'dashline', label: 'Dependency' }, ...extra];
const K = klass;

// ---------- store ports ----------
{
  const d = P({ title: 'Domain model — the Store ports', kind: 'UML · Class diagram (1 of 4)', w: 1376,
    desc: 'One `Store` per window. The shell codes against the ports; the DuckDB store, its remote client and the in-memory oracle implement them.' });
  const W = 352, G = 96, x = (i) => 48 + i * (W + G);
  const sw = 3 * W + 2 * G;
  K(d, 'store', 48, Y0, sw, { name: 'Store', stereotype: 'interface · ports (debrief_core::ports)', role: 'accent',
    attrs: ['Ingest · Query · Positions · Analysis · Profiles', 'Diagnostics · Search · Projects · Inspect · Maintain', 'Console · Redaction · Skew · Tail  (fourteen traits)'] });
  const y1 = Y0 + d.rect.store[3] + 72;
  K(d, 'ws', x(0), y1, W, { name: 'WorkspaceStore', stereotype: 'adapter · test oracle', attrs: ['− Workspace: RwLock', '  records, sources, caches in memory'], ops: ['+ new(Workspace): WorkspaceStore', '+ read() · write(): lock guard'] });
  K(d, 'duck', x(1), y1, W, { name: 'DuckStore', stereotype: 'adapter · debrief-store-duck', attrs: ['− Parquet segments, DuckDB catalog', '− Tantivy full-text index', '− programs compiled to SQL'], ops: ['+ open(dir, Profile, PluginRegistry)'] });
  K(d, 'rem', x(2), y1, W, { name: 'RemoteStore', stereotype: 'adapter · debrief-store-client', attrs: ['− StoreWorker (supervised child)', '− request frame per port method'], ops: ['impl Port for RemoteStore', '  expanded from store_port_table!'] });
  const y2 = Math.max(...['ws', 'duck', 'rem'].map((i) => bottom(d, i))) + 72;
  K(d, 'wst', x(0), y2, W, { name: 'WorkspaceState', attrs: ['+ roots, removed: Vec<String>', '+ offsets: ClockOffsets'] });
  K(d, 'clk', x(1), y2, W, { name: 'ClockOffsets', stereotype: 'newtype', attrs: ['BTreeMap<String, i64>', '  source path → offset (ms)'], ops: ['+ get(path): i64', '+ set(path, ms): Result'] });
  K(d, 'man', x(2), y2, W, { name: 'ProjectManifest', attrs: ['+ format, version, name', '+ profile_overrides: String (TOML)', '+ clock_offsets: Vec<SourceOffset>', '+ roots: Vec<String>', '+ parsers: Vec<ParserRef>'] });
  K(d, 'off', x(2), y2 + d.rect.man[3] + 56, W, { name: 'SourceOffset', attrs: ['+ path: String', '+ offset_ms: i64'] });
  for (const [a, g] of [['ws', 0], ['duck', 1], ['rem', 2]]) realize(d, a, 'T', 'store', 'B', { t: [0.5, (x(g) + W / 2 - 48) / sw] });
  link(d, 'ws', 'B', 'wst', 'T', { dash: true, arrow: 'open', label: 'state()' });
  link(d, 'wst', 'R', 'clk', 'L', { ...comp, label: '1' });
  link(d, 'man', 'B', 'off', 'T', { ...comp, label: '0..*' });
  d.legend([{ kind: 'accent', label: 'Port or adapter' }, { kind: 'dashline', label: 'Realisation' }, { kind: 'line', label: 'Association (diamond: composition)' }]);
  save('uml-class-domain-store', d, bottom(d, 'wst', 'clk', 'off'));
}

// ---------- events and sources ----------
{
  const d = P({ title: 'Domain model — events and sources', kind: 'UML · Class diagram (2 of 4)', w: 1472,
    desc: 'What an event is while it is parsed (`Record`), what the store keeps (`EventBatch`) and what the window learns about its sources (`WorkspaceSummary`).' });
  const W = 304, G = 104, x = (i) => 48 + i * (W + G);
  K(d, 'rec', x(0), Y0, W, { name: 'Record', attrs: ['+ id: u32 · ts: Option<i64> (ms)', '+ level: Level', '+ message: String', '+ template, exception: Option', '+ fields: Fields (JSON map)', '+ source: u32 · line: u32', '+ markers: MarkerMask (u64)'], ops: ['+ field_text(name): Option<Cow>', '+ field_number(name): Option<f64>'] });
  K(d, 'lvl', x(0), Y0 + d.rect.rec[3] + 72, W, { name: 'Level', stereotype: 'enumeration', role: 'core', attrs: ['Verbose · Debug · Information', 'Warning · Error · Fatal'], ops: ['+ parse(&str) · as_str()'] });
  K(d, 'bat', x(1), Y0, W, { name: 'EventBatch', stereotype: 'debrief-batch · Arrow', attrs: ['− schema: Arc<EventSchema>', '− batch: RecordBatch', '  at most 8192 rows, ids assigned'], ops: ['+ try_new(…): Result', '+ row(i): BatchRow'] });
  K(d, 'sch', x(1), Y0 + d.rect.bat[3] + 72, W, { name: 'EventSchema', attrs: ['fixed columns (id, ts, level, …)', 'declared columns of a profile', 'other fields: the `extra` map'], ops: ['+ plain() · with_declared(…)'] });
  K(d, 'sum', x(2), Y0, W, { name: 'WorkspaceSummary', attrs: ['+ sources: Vec<SourceInfo>', '+ unrecognized: Vec', '+ total · untimed · from · to', '+ levels: Map<Level, usize>', '+ fields: Vec<FieldInfo>', '+ marker_kinds · charts', '+ pipeline: Vec<PipelineStepInfo>', '+ plugins · exporters · dashboards', '+ lenses: Vec<LensCfg>', '+ profile_name · load_ms'] });
  const cx = x(3);
  K(d, 'src', cx, Y0, W, { name: 'SourceInfo', attrs: ['+ id: u32 · path · name · group', '+ importer: String', '+ records · skipped · dropped', '+ step_stats: Vec<StepStats>', '+ from, to: Option<i64>', '+ offset_ms: i64', '+ file_kind · timestamp_issue', '+ error: Option<String>'] });
  K(d, 'stp', cx, bottom(d, 'src') + 48, W, { name: 'StepStats', attrs: ['+ input, output, errors: usize'] });
  K(d, 'unr', cx, bottom(d, 'stp') + 48, W, { name: 'UnrecognizedFile', attrs: ['+ path, name, reason: String', '+ crashed: bool'] });
  K(d, 'fld', cx, bottom(d, 'unr') + 48, W, { name: 'FieldInfo', attrs: ['+ name: String', '+ count, distinct: usize', '+ numeric, derived: bool'] });
  link(d, 'rec', 'B', 'lvl', 'T', { arrow: 'open', label: 'level' });
  link(d, 'bat', 'B', 'sch', 'T', { arrow: 'open', label: '1' });
  link(d, 'bat', 'L', 'rec', 'R', { dash: true, arrow: 'open', label: 'to_record' });
  // summary owns three lists; one trunk
  const tx = x(2) + W + 48, sy = d.rect.sum[1] + 32;
  for (const [id, lab] of [['src', '0..*'], ['unr', '0..*'], ['fld', '0..*']]) {
    const cy = Math.round(d.rect[id][1] + 24);
    d.edge('sum', id, [x(2) + W, sy], [cx, cy], { ...comp, via: [[tx, sy], [tx, cy]] });
    d.lbl(lab, cx - 8, cy, 'bl');
  }
  link(d, 'src', 'B', 'stp', 'T', { ...comp, label: '0..*' });
  d.legend([{ kind: 'accent', label: 'Type' }, { kind: 'core', label: 'Enumeration' }, { kind: 'line', label: 'Association (diamond: composition)' }, { kind: 'dashline', label: 'Dependency' }]);
  save('uml-class-domain-events', d, bottom(d, 'fld'));
}

// ---------- profile and parsers ----------
{
  const d = P({ title: 'Domain model — profile and parsers', kind: 'UML · Class diagram (3 of 4)', w: 1472,
    desc: 'The profile is data (TOML merged over `default_profile.toml`, which documents every key); compiling it yields the pipeline that ingest runs.' });
  const W = 328, G = 96, x = (i) => 48 + i * (W + G);
  K(d, 'pro', x(0), Y0, W, { name: 'Profile', attrs: ['+ name: String', '+ files: FilesCfg', '+ instance: InstanceCfg', '+ activity: ActivityCfg', '+ extract: Vec<ExtractCfg>', '+ marker: Vec<MarkerCfg>', '+ chart: Vec<ChartCfg>', '+ pipeline: Vec<StepCfg>', '+ icon: Vec<IconCfg>', '+ live: LiveCfg', '+ redact: RedactCfg', '+ dashboard: Vec<DashboardCfg>', '+ lens: Vec<LensCfg>', '+ span: Vec<SpanCfg>', '+ route: Vec<RouteCfg>'], ops: ['+ from_toml(&str): Result', '+ default_profile(): Profile'] });
  K(d, 'len', x(0), bottom(d, 'pro') + 56, W, { name: 'LensCfg', attrs: ['+ id, title, description: String', '+ levels: Vec<Level>', '+ text: String · regex: bool', '+ ql: String', '+ markers: Vec<String>', '+ filter: Vec<FieldFilter>', '+ columns: Vec<String>', '+ highlight: Vec<LensHighlight>', '+ chart: Option<String>'] });
  K(d, 'cp', x(1), Y0, W, { name: 'CompiledProfile', attrs: ['+ profile: Profile', '+ format: FormatCfg', '+ parser: ParserRef', '− include, exclude: GlobSet', '− timestamps: TimestampParser', '− extractors: Vec<Extractor>', '− markers: Vec<Marker>', '− pipeline, format_pipeline', '− spans · routes · icons'], ops: ['+ compile(Profile): Result', '+ ingest(records, source_name): Loaded'] });
  K(d, 'pip', x(1), bottom(d, 'cp') + 72, W, { name: 'Pipeline', attrs: ['− steps: Vec<(Vec<Condition>, Step)>', '− calc_fields: Vec<String>'], ops: ['+ compile(&[StepCfg], &transforms)', '+ run_with_stats(records, source)'] });
    K(d, 'set', x(2), Y0, W, { name: 'ParserSet', attrs: ['the parser library, compiled', 'with the profile (ADR 0026)'], ops: ['+ choose(file head, name): Parser'] });
  K(d, 'par', x(2), bottom(d, 'set') + 72, W, { name: 'Parser', attrs: ['+ schema: u32 · id · name · version', '+ category · description', '+ matching: MatchCfg', '+ format: FormatCfg', '+ extract: Vec<ExtractCfg>', '+ pipeline: Vec<StepCfg>'] });
  K(d, 'fmt', x(2), bottom(d, 'par') + 72, W, { name: 'FormatCfg', attrs: ['+ kind: String (auto, text, json, …)', '+ identity · output_templates', '+ regexes · timestamp_formats', '+ assume_timezone · day_first', '+ json: JsonCfg · csv: CsvCfg'] });
  const y3 = bottom(d, 'fmt') + 72;
  K(d, 'stp', x(1), y3, W, { name: 'Step', stereotype: 'enumeration · private', role: 'core', attrs: ['Transform(Arc<dyn RecordTransform>)', 'Dedupe { fields, window_ms }', 'Script(Arc<Script>)'] });
  K(d, 'rt', x(2), y3, W, { name: 'RecordTransform', stereotype: 'interface · plugin contribution', attrs: ['+ id(): &str', '+ apply(&mut Record, ctx): Verdict'] });
  link(d, 'cp', 'L', 'pro', 'R', { arrow: 'open', label: 'profile' });
  link(d, 'cp', 'B', 'pip', 'T', { ...comp, label: '1' });
  link(d, 'pip', 'B', 'stp', 'T', { ...comp, label: '0..*' });
  link(d, 'stp', 'R', 'rt', 'L', { dash: true, arrow: 'open', label: 'Transform' });
  link(d, 'pro', 'B', 'len', 'T', { ...comp, label: '0..*' });
  link(d, 'set', 'B', 'par', 'T', { ...comp, label: '0..*' });
  link(d, 'par', 'B', 'fmt', 'T', { ...comp, label: '1' });
  d.legend(LEG());
  save('uml-class-domain-profile', d, bottom(d, 'len', 'rt', 'stp'));
}

// ---------- queries ----------
{
  const d = P({ title: 'Domain model — queries and filters', kind: 'UML · Class diagram (4 of 4)', w: 1472,
    desc: 'A `Query` is what the UI sends; it compiles to a typed expression tree. The store turns the same tree into SQL; the oracle evaluates it directly.' });
  const W = 328, G = 112, x = (i) => 48 + i * (W + G);
  K(d, 'qry', x(0), Y0, W, { name: 'Query', attrs: ['+ from, to: Option<i64>', '+ levels: Vec<Level>', '+ text: String · regex · case_sensitive', '+ text_mode: TextMode', '+ filters: Vec<FieldFilter>', '+ group_by · groups: Vec<String>', '+ markers: Vec<String>', '+ ql: String', '+ window: Option<TimeWindow>', '+ sources: Vec<u32> (≤ 10 000)'] });
  K(d, 'ff', x(0), bottom(d, 'qry') + 72, W, { name: 'FieldFilter', attrs: ['+ field, op: String', '+ value: Option<String>'] });
  K(d, 'cq', x(1), Y0, W, { name: 'CompiledQuery', attrs: ['− from, to: Option<i64>', '+ group_by: Vec<String>', '− structural: Vec<Expr>', '  sources, time, field filters', '− content: Vec<Expr>', '  levels, markers, search, ql'], ops: ['+ new(&Query, group_by, kinds): Result'] });
  K(d, 'prg', x(1), bottom(d, 'cq') + 72, W, { name: 'ql::Program', attrs: ['+ filter: Filter (Option<Expr>)', '+ stages: Vec<Stage>', '+ field_refs: Vec<(usize, FieldRef)>'], ops: ['ql::parse(&str): Result<Program>'] });
  K(d, 'stg', x(1), bottom(d, 'prg') + 72, W, { name: 'ql::Stage', stereotype: 'enumeration', role: 'core', attrs: ['Where(Filter) · Head(usize)', 'Stats { aggs, by } · Sort', 'Fields(Vec<FieldRef>)'] });
  K(d, 'exp', x(2), Y0, W, { name: 'Expr', stereotype: 'enumeration', role: 'core', attrs: ['And · Or · Not · Const(bool)', 'Cmp { field, op, rhs }', 'Contains · Regex · In', 'Exists · Missing', 'Text · TextEq · TextIn · NumCmp', 'Search(TextSearch) · Levels(u8)'] });
  K(d, 'cnd', x(2), bottom(d, 'exp') + 72, W, { name: 'Condition', attrs: ['− check: Check', '− expr: Expr'], ops: ['+ compile(&ConditionCfg): Result', '+ eval(&Record, source): bool', '+ eval_row(&dyn RowRef): bool'] });
  K(d, 'cfg', x(2), bottom(d, 'cnd') + 72, W, { name: 'ConditionCfg', attrs: ['+ field, op: String', '+ value: Option<CondValue>'] });
  link(d, 'qry', 'B', 'ff', 'T', { ...comp, label: '0..*' });
  link(d, 'cq', 'L', 'qry', 'R', { dash: true, arrow: 'open', label: 'compiled from' });
  link(d, 'cq', 'R', 'exp', 'L', { ...comp, label: '0..*' });
  link(d, 'prg', 'B', 'stg', 'T', { ...comp, label: '0..*' });
  link(d, 'cnd', 'T', 'exp', 'B', { arrow: 'open', label: 'expr' });
  link(d, 'cnd', 'B', 'cfg', 'T', { dash: true, arrow: 'open', label: 'compiled from' });
  d.legend(LEG());
  save('uml-class-domain-query', d, bottom(d, 'ff', 'cfg', 'stg'));
}

// ---------- plugin API: the plugin side ----------
{
  const d = P({ title: 'Plugin API — what a plugin implements', kind: 'UML · Class diagram (1 of 3)', w: 1472,
    desc: 'A plugin depends on `debrief-plugin-api` alone. Its `register` adds contributions; `serve` answers the host over `debrief-ipc` in a sandboxed process (ADR 0017).' });
  const W = 328, G = 96, x = (i) => 48 + i * (W + G);
  K(d, 'srv', x(0), Y0, W, { name: 'serve', stereotype: 'function', attrs: ['connect to the host, lock down,', 'answer until the host closes'], ops: ['serve(Arc<dyn Plugin>): io::Result'] });
  K(d, 'plg', x(0), bottom(d, 'srv') + 72, W, { name: 'Plugin', stereotype: 'interface · debrief-plugin-api', attrs: ['+ id(), name(), version(): &str'], ops: ['+ register(&self, &mut Contributions)', '+ permissions(): Vec<PermissionRequest>', '+ memory_limit(): Option<u64>'] });
  K(d, 'con', x(0), bottom(d, 'plg') + 72, W, { name: 'Contributions', attrs: ['− text: Vec<Arc<dyn TextImporter>>', '− binary · profile_importers', '− document_importers', '− config_parsers · annotators', '− transforms · tools', '− marker_kinds · settings'] });
  const gx = x(1) - 24, gw = 2 * W + G + 48;
  // contribution traits, two columns
  const T = (id, col, y, spec) => K(d, id, x(1) + col * (W + G), y, W, { stereotype: 'interface', ...spec });
  const r0 = Y0 + 40;
  T('ti', 0, r0, { name: 'TextImporter', attrs: ['+ id(): &str'], ops: ['+ sniff(ctx, head: &str): u8', '+ import(ctx, content): Result<Imported>', '+ import_tail(ctx, head, tail)', '+ multiline(): bool'] });
  T('tr', 1, r0, { name: 'Transform', attrs: ['+ id(): &str'], ops: ['+ apply(&mut Record, ctx): Verdict'] });
  const r1 = r0 + Math.max(d.rect.ti[3], d.rect.tr[3]) + 40;
  T('bi', 0, r1, { name: 'BinaryImporter', attrs: ['+ id(): &str'], ops: ['+ sniff(ctx, head: &[u8]): u8', '+ import(ctx, bytes): Result<Imported>'] });
  T('an', 1, r1, { name: 'Annotator', attrs: ['+ id(): &str'], ops: ['+ annotate(view, since): Result<Vec>'] });
  const r2 = r1 + Math.max(d.rect.bi[3], d.rect.an[3]) + 40;
  T('pi', 0, r2, { name: 'ProfileImporter · DocumentImporter', attrs: ['+ id(): &str', '+ sniff(file_name, head): u8'], ops: ['+ import_encoded(file_name, bytes, …)'] });
  T('tl', 1, r2, { name: 'Tool', attrs: ['+ id(): &str', '+ describe(): ToolInfo'], ops: ['+ invoke(op, args, &ToolCtx): Result', '+ stop(task, ctx)'] });
  const r3 = r2 + Math.max(d.rect.pi[3], d.rect.tl[3]) + 40;
  T('cp', 0, r3, { name: 'ConfigParser', attrs: ['+ id(), name(), extensions()'], ops: ['+ sniff(file_name, head): u8', '+ parse(content, limits): ConfigTree'] });
  T('cf', 1, r3, { name: 'Configurable', attrs: ['+ settings_section(): SettingsSection'], ops: ['+ apply_settings(&ValidValues): Result'] });
  const gh = r3 + Math.max(d.rect.cp[3], d.rect.cf[3]) + 24 - Y0;
  d.group('traits', gx, Y0, gw, gh, 'Contribution traits (debrief_plugin_api)');
  // move the group frame behind the class boxes
  const frame = d.cells.pop();
  d.cells.unshift(frame);
  link(d, 'srv', 'B', 'plg', 'T', { dash: true, arrow: 'open', label: 'serves' });
  link(d, 'plg', 'B', 'con', 'T', { dash: true, arrow: 'open', label: 'fills' });
  link(d, 'con', 'R', 'traits', 'L', { ...comp, label: '0..*', t: [0.5, (d.rect.con[1] + d.rect.con[3] / 2 - Y0) / gh] });
  d.legend([{ kind: 'accent', label: 'Type or trait' }, { kind: 'line', label: 'Association (diamond: composition)' }, { kind: 'dashline', label: 'Dependency' }]);
  save('uml-class-plugin-api', d, Y0 + gh);
}

// ---------- plugin API: the host side ----------
{
  const d = P({ title: 'Plugin API — registry and guards in the engine', kind: 'UML · Class diagram (2 of 3)', w: 1472,
    desc: 'Every contribution reaches the engine wrapped in a `Guarded*` adapter: panics become errors attributed to the plugin and repeat offenders are quarantined.' });
  const W = 328, G = 112, x = (i) => 48 + i * (W + G);
  K(d, 'plg', x(0), Y0, W, { name: 'Plugin', stereotype: 'interface · debrief_core::plugin', attrs: ['+ id(), name(), version(): &str', '+ api_version(): u32 = API_VERSION'], ops: ['+ register(&self, &mut PluginRegistry)', '+ manifest(): Manifest', '+ activate(&ActivationCtx) · deactivate()'] });
  K(d, 'con', x(2), Y0, W, { name: 'Contribution traits', stereotype: 'interface · debrief_core', attrs: ['Importer · BinaryImporter', 'RecordTransform · Exporter', 'Collector · Annotator', 'WorkspaceHook'] });
  const y2 = bottom(d, 'plg') + 72;
  K(d, 'reg', x(0), y2, W, { name: 'PluginRegistry', attrs: ['− importers: Importers', '− transforms · hooks · exporters', '− collectors · annotators · tools', '− runtime: Health, EventBus, services', '− disabled: HashSet<String>'], ops: ['+ install_shared(Arc<dyn Plugin>)', '+ add_importer · add_transform · …', '+ disable(ids) · resolve() · shutdown()', '+ plugins(): Vec<PluginInfo>'] });
  K(d, 'grd', x(1), y2, W, { name: 'Guard', attrs: ['− owner: plugin id', '− health: Arc<Health>'], ops: ['+ call(kind: CallKind, what, f): Result', '+ call_detached(kind, what, f)', 'guard::catch(what, f): panic → Err'] });
  K(d, 'gua', x(2), y2, W, { name: 'Guarded*', stereotype: 'adapter', attrs: ['GuardedImporter · GuardedBinaryImporter', 'GuardedTransform · GuardedExporter', 'GuardedHook · GuardedCollector', 'GuardedConfigurable', '− inner: Arc<dyn …> · guard: Guard'] });
  const y3 = Math.max(bottom(d, 'reg'), bottom(d, 'grd'), bottom(d, 'gua')) + 72;
  K(d, 'inf', x(0), y3, W, { name: 'PluginInfo', attrs: ['+ id, name, version: String', '+ status: PluginStatus', '+ failures: u32 · last_error', '+ requires · provides · dependents', '+ inactive_reason · warnings · hung'] });
  K(d, 'hlt', x(1), y3, W, { name: 'Health', attrs: ['QUARANTINE_AFTER: u32 = 3'], ops: ['+ admit(plugin, kind): bool', '+ fail(plugin) · mark_hung(plugin)', '+ reset(plugin): lifts a quarantine'] });
  K(d, 'sta', x(0), bottom(d, 'inf') + 56, W, { name: 'PluginStatus', stereotype: 'enumeration', role: 'core', attrs: ['Active · Disabled', 'Quarantined · Inactive'] });
  link(d, 'reg', 'T', 'plg', 'B', { dash: true, arrow: 'open', label: 'install' });
  link(d, 'reg', 'R', 'grd', 'L', { ...comp, label: 'wraps with' });
  link(d, 'grd', 'R', 'gua', 'L', { ...comp, label: '1' });
  link(d, 'gua', 'T', 'con', 'B', { dash: true, arrow: 'open', label: 'realises' });
  link(d, 'grd', 'B', 'hlt', 'T', { arrow: 'open', label: 'health' });
  link(d, 'reg', 'B', 'inf', 'T', { dash: true, arrow: 'open', label: 'plugins()' });
  link(d, 'inf', 'B', 'sta', 'T', { arrow: 'open', label: 'status' });
  d.legend([{ kind: 'accent', label: 'Type or trait' }, { kind: 'core', label: 'Enumeration' }, { kind: 'line', label: 'Association (diamond: composition)' }, { kind: 'dashline', label: 'Dependency' }]);
  save('uml-class-plugin-registry', d, bottom(d, 'hlt', 'sta'));
}

// ---------- plugin API: out of process ----------
{
  const d = P({ title: 'Plugin API — plugins out of process', kind: 'UML · Class diagram (3 of 3)', w: 1472,
    desc: 'Built-in and third-party plugins run the same way: `RemotePlugin` is a `Plugin` whose contributions are proxies to a supervised, sandboxed child (ADR 0017, 0027).' });
  const W = 328, G = 112, x = (i) => 48 + i * (W + G);
  K(d, 'bui', x(0), Y0, W, { name: 'Builtins', attrs: ['− processes: Vec<Arc<PluginProcess>>', '− problem: Option<String>'], ops: ['+ load(exe) · none() (safe mode)', '+ add_installed(process)', '+ plugins(): Vec<Arc<dyn Plugin>>', '+ warm_up() · shutdown()'] });
  K(d, 'dir', x(0), bottom(d, 'bui') + 72, W, { name: 'PluginsDir', attrs: ['− root: PathBuf'], ops: ['+ record(id): Option<InstallRecord>', '+ home(record): PluginHome'] });
  K(d, 'rem', x(1), Y0, W, { name: 'RemotePlugin', attrs: ['− process: Arc<PluginProcess>'], ops: ['+ new(process) · process()', '+ register(registry): proxies'] });
  K(d, 'rx', x(1), bottom(d, 'rem') + 72, W, { name: 'Remote*', stereotype: 'adapter', attrs: ['RemoteImporter · RemoteBinaryImporter', 'RemoteTransform (batched)', 'RemoteAnnotator · RemoteTool', 'RemoteConfigParser · RemoteSettings', 'deadline: 0.9 × the call budget'] });
  K(d, 'prc', x(2), Y0, W, { name: 'PluginProcess', attrs: ['− spec: PluginSpec (exe, args, granted)', '− supervisor: Supervisor', '− limits: PluginLimits'], ops: ['+ call(…) · reset() · shutdown()', '+ warm_up() · granted(permission)'] });
  K(d, 'sup', x(2), bottom(d, 'prc') + 72, W, { name: 'Supervisor', stereotype: 'debrief-ipc', attrs: ['max_crashes in crash_window', 'backoff_base … backoff_max'], ops: ['+ lease(): restarts lazily', '+ status(): ChildStatus'] });
  K(d, 'chd', x(2), bottom(d, 'sup') + 72, W, { name: 'debrief-plugins --plugin <id>', stereotype: 'child process', role: 'process', attrs: ['debrief_plugin_api::serve(plugin)', 'postcard frames, private local', 'stream; sandbox from manifest'] });
  link(d, 'bui', 'R', 'rem', 'L', { ...comp, label: '0..*' });
  link(d, 'rem', 'R', 'prc', 'L', { arrow: 'open', label: 'process' });
  link(d, 'rem', 'B', 'rx', 'T', { dash: true, arrow: 'open', label: 'registers' });
  link(d, 'prc', 'B', 'sup', 'T', { ...comp, label: '1' });
  link(d, 'sup', 'B', 'chd', 'T', { dash: true, arrow: 'open', label: 'starts' });
  link(d, 'dir', 'T', 'bui', 'B', { dash: true, arrow: 'open', label: 'adds' });
  d.legend([{ kind: 'accent', label: 'Type or trait' }, { kind: 'process', label: 'Child process' }, { kind: 'line', label: 'Association (diamond: composition)' }, { kind: 'dashline', label: 'Dependency' }]);
  save('uml-class-plugin-process', d, bottom(d, 'dir', 'rx', 'chd'));
}
