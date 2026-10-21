import { Level, SourceProblem, TimestampIssue } from '../api.types';

/** A raw imported event, before the profile pipeline (extractors, markers) runs. */
export interface MockRecord {
  id: number;
  ts: number | null;
  level: Level;
  message: string;
  template: string | null;
  exception: string | null;
  fields: Record<string, unknown>;
  source: number;
  line: number;
  markers: string[];
}

export interface MockSource {
  id: number;
  path: string;
  name: string;
  importer: string;
  skipped: number;
  /** Clock-skew offset applied to the source's timestamps (ms). */
  offsetMs?: number;
  /** The engine's timestamp diagnosis, mimicked (see `mock-timestamps.ts`). */
  timestampIssue?: TimestampIssue | null;
  /** The file a profile route split this logical source from (see `SourceInfo.routedFrom`). */
  routedFrom?: string | null;
  /** Import problems, as the engine reports them. */
  problems?: SourceProblem[];
}

export interface MockDataset {
  sources: MockSource[];
  unrecognized: { path: string; name: string; reason: string }[];
  records: MockRecord[];
}

const SEC = 1000;
const MIN = 60 * SEC;
export const DEMO_START = Date.UTC(2026, 9, 14, 9, 0, 0);
const DEMO_END = DEMO_START + 3 * 60 * MIN;
const CRASH = Date.UTC(2026, 9, 14, 10, 41, 24, 50);
const RESTART = Date.UTC(2026, 9, 14, 10, 42, 10, 412);
const INCIDENT = Date.UTC(2026, 9, 14, 10, 41, 0);

function mulberry32(seed: number): () => number {
  let a = seed;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Renders a Serilog message template the way {Message:lj} does. */
export function renderTemplate(template: string, props: Record<string, unknown>): string {
  return template.replace(/\{\{|\}\}|\{([@$]?)([A-Za-z0-9_]+)(?:[,:][^}]*)?\}/g, (all, _op, name: string) => {
    if (all === '{{') return '{';
    if (all === '}}') return '}';
    const v = props[name];
    if (v === undefined) return all;
    return typeof v === 'string' ? v : JSON.stringify(v);
  });
}

const TIMEOUT_STACK = `System.TimeoutException: The operation has timed out.
   at Microsoft.Data.SqlClient.SqlInternalConnection.OnError(SqlException exception, Boolean breakConnection)
   at Microsoft.Data.SqlClient.TdsParser.ThrowExceptionAndWarning(TdsParserStateObject stateObj)
   at Microsoft.EntityFrameworkCore.Query.Internal.SingleQueryingEnumerable\`1.AsyncEnumerator.MoveNextAsync()
   at OrderService.Payments.CaptureHandler.Handle(CaptureCommand cmd, CancellationToken ct) in /src/Payments/CaptureHandler.cs:line 88`;

const HTTP_STACK = `System.Net.Http.HttpRequestException: The request was canceled due to the configured HttpClient.Timeout of 5 seconds elapsing.
 ---> System.TimeoutException: The operation was canceled.
   at System.Net.Http.HttpClient.HandleFailure(Exception e, Boolean telemetryStarted, HttpResponseMessage response)
   at OrderService.Payments.GatewayClient.CaptureAsync(Guid orderId, CancellationToken ct) in /src/Payments/GatewayClient.cs:line 41`;

interface Instance {
  app: string;
  machine: string;
  pid: number;
  source: number;
  start: number;
  end: number;
  kind: 'web' | 'worker' | 'gateway' | 'batch';
  ratePerMin: number;
  crash?: boolean;
  stop?: boolean;
  cpuBase: number;
  wsBase: number;
  leak?: boolean;
}

/** Builds a realistic multi-instance incident: a payment timeout storm that crashes WEB02. */
export function generateDemo(seed = 42): MockDataset {
  const rnd = mulberry32(seed);
  const pick = <T>(xs: T[]): T => xs[Math.floor(rnd() * xs.length)] as T; // callers pass non-empty lists
  const guid = () =>
    'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
      const r = Math.floor(rnd() * 16);
      return (c === 'x' ? r : (r & 0x3) | 0x8).toString(16);
    });

  const sources = [
    { id: 0, path: '/incident-2026-10-14/web01/orders-20261014.clef', name: 'orders-20261014.clef', importer: 'json', skipped: 0 },
    { id: 1, path: '/incident-2026-10-14/web02/orders-20261014.clef', name: 'orders-20261014.clef', importer: 'json', skipped: 0 },
    { id: 2, path: '/incident-2026-10-14/worker01/worker-20261014.log', name: 'worker-20261014.log', importer: 'text', skipped: 0 },
    { id: 3, path: '/incident-2026-10-14/gateway/gateway-20261014.json', name: 'gateway-20261014.json', importer: 'json', skipped: 0 },
    { id: 4, path: '/incident-2026-10-14/sync/inventory-sync.csv', name: 'inventory-sync.csv', importer: 'csv', skipped: 2 },
    { id: 5, path: '/incident-2026-10-14/system/registry-export.reg', name: 'registry-export.reg', importer: 'registry', skipped: 0 },
  ];

  const instances: Instance[] = [
    { app: 'OrderService', machine: 'WEB01', pid: 3312, source: 0, start: DEMO_START + 2 * SEC, end: DEMO_END, kind: 'web', ratePerMin: 55, cpuBase: 45, wsBase: 860 },
    { app: 'OrderService', machine: 'WEB02', pid: 4120, source: 1, start: DEMO_START + 5 * SEC, end: CRASH, kind: 'web', ratePerMin: 55, crash: true, cpuBase: 40, wsBase: 640, leak: true },
    { app: 'OrderService', machine: 'WEB02', pid: 5532, source: 1, start: RESTART, end: DEMO_END, kind: 'web', ratePerMin: 50, cpuBase: 32, wsBase: 420 },
    { app: 'OrderService.Worker', machine: 'WORKER01', pid: 2208, source: 2, start: DEMO_START + 1 * SEC, end: DEMO_END, kind: 'worker', ratePerMin: 45, cpuBase: 26, wsBase: 610 },
    { app: 'OrderService.Worker', machine: 'WORKER01', pid: 2210, source: 2, start: DEMO_START + 30 * MIN, end: DEMO_START + 75 * MIN, kind: 'batch', ratePerMin: 30, stop: true, cpuBase: 60, wsBase: 300 },
    { app: 'Gateway', machine: 'GATEWAY', pid: 1880, source: 3, start: DEMO_START + 1 * SEC, end: DEMO_END, kind: 'gateway', ratePerMin: 50, cpuBase: 22, wsBase: 350 },
  ];

  const records: MockRecord[] = [];
  const lines = new Map<number, number>();

  const emit = (
    inst: { app: string; machine: string; pid: number; source: number },
    ts: number,
    level: Level,
    sourceContext: string,
    template: string,
    props: Record<string, unknown> = {},
    exception: string | null = null,
  ) => {
    const line = (lines.get(inst.source) ?? 0) + 1;
    lines.set(inst.source, line + (exception ? exception.split('\n').length : 0));
    const fields: Record<string, unknown> = {
      ...props,
      SourceContext: sourceContext,
      Application: inst.app,
      MachineName: inst.machine,
      ProcessId: inst.pid,
      ThreadId: 4 + Math.floor(rnd() * 60),
      EnvironmentName: 'Production',
    };
    records.push({
      id: 0,
      ts,
      level,
      message: renderTemplate(template, props),
      template,
      exception,
      fields,
      source: inst.source,
      line,
      markers: [],
    });
  };

  for (const inst of instances) {
    // Lifecycle: start.
    emit(inst, inst.start, 'Information', 'Program', 'Application starting — {Application} {Version} ({Framework})', {
      Application: inst.app,
      Version: inst.pid === 5532 ? '3.14.2' : '3.14.1',
      Framework: 'net9.0',
    });
    if (inst.kind === 'web' || inst.kind === 'gateway') {
      emit(inst, inst.start + 640, 'Information', 'Microsoft.Hosting.Lifetime', 'Now listening on: {Address}', {
        Address: 'http://0.0.0.0:8080',
      });
    }
    if (inst.kind === 'batch') {
      emit(inst, inst.start + 200, 'Information', 'Reindex.Job', 'Nightly reindex started for {IndexCount} indexes', { IndexCount: 14 });
    }

    // Process stats once a minute.
    for (let t = inst.start + 30 * SEC; t < inst.end; t += MIN) {
      const p = (t - inst.start) / (DEMO_END - DEMO_START);
      const near = Math.exp(-Math.pow((t - INCIDENT) / (6 * MIN), 2));
      let cpu = inst.cpuBase + 8 * Math.sin(t / (13 * MIN)) + rnd() * 7 + near * 22;
      if (inst.leak) cpu = inst.cpuBase + p * 120 + rnd() * 5;
      let ws = inst.wsBase + 30 * Math.sin(t / (17 * MIN)) + rnd() * 20;
      if (inst.leak) ws = inst.wsBase + p * 3200 + rnd() * 25;
      cpu = Math.min(99.4, Math.max(1, cpu));
      const big = ws >= 1024;
      emit(inst, t, 'Information', 'Diagnostics.ProcessMonitor', 'Process stats CPU {Cpu}% · WorkingSet {WorkingSet} · Threads {Threads}', {
        Cpu: cpu.toFixed(1),
        WorkingSet: big ? `${(ws / 1024).toFixed(2)} GB` : `${Math.round(ws)} MB`,
        Threads: Math.round(40 + cpu * 1.6 + rnd() * 10),
      });
    }

    // Traffic.
    let t = inst.start + 1500;
    while (t < inst.end - 2 * SEC) {
      const near = Math.exp(-Math.pow((t - INCIDENT) / (4 * MIN), 2));
      const rate = inst.ratePerMin * (1 + near * 1.5);
      t += Math.max(15, (-Math.log(1 - rnd()) * MIN) / rate);
      if (t >= inst.end - 2 * SEC) break;
      const r = rnd();
      const order = 88000 + Math.floor(rnd() * 600);
      const cid = guid();
      if (inst.kind === 'web') {
        if (r < near * 0.12 + 0.0008) {
          emit(inst, t, 'Error', 'OrderService.Payments', 'Failed to capture payment for order {OrderId}: request timed out', { OrderId: order, CorrelationId: cid, RequestPath: `/api/orders/${order}/capture` }, HTTP_STACK);
        } else if (r < near * 0.2 + 0.003) {
          emit(inst, t, 'Warning', 'OrderService.Payments', 'Payment gateway call exceeded {TimeoutMs} ms (attempt {Attempt}/3) for order {OrderId}', { TimeoutMs: 5000, Attempt: 1 + Math.floor(rnd() * 3), OrderId: order, CorrelationId: cid });
        } else if (r < near * 0.24 + 0.0035) {
          emit(inst, t, 'Error', 'Microsoft.EntityFrameworkCore.Query', "An exception occurred while iterating over the results of a query for context type '{contextType}'.", { contextType: 'OrderService.Data.OrdersDbContext', CorrelationId: cid, RequestPath: `/api/orders/${order}/capture` }, TIMEOUT_STACK);
        } else if (r < near * 0.24 + 0.005) {
          emit(inst, t, 'Error', 'OrderService.Data', 'Lock request timed out on {Table}; transaction rolled back (retry {Retry}/3)', { Table: 'dbo.Orders', Retry: 1 + Math.floor(rnd() * 3), CorrelationId: cid }, TIMEOUT_STACK);
        } else if (r < 0.012) {
          emit(inst, t, 'Warning', 'OrderService.Orders', 'Slow request {RequestPath} took {ElapsedMs} ms', { RequestPath: `/api/orders/${order}`, ElapsedMs: 1500 + Math.floor(rnd() * 4000), CorrelationId: cid });
        } else if (r < 0.25) {
          emit(inst, t, 'Debug', 'OrderService.Caching', 'Cache {Outcome} for {CacheKey}', { Outcome: pick(['hit', 'hit', 'miss']), CacheKey: `order:${order}` });
        } else if (r < 0.45) {
          emit(inst, t, 'Information', 'OrderService.Orders', 'Order {OrderId} placed by customer {CustomerId}', { OrderId: order, CustomerId: 1000 + Math.floor(rnd() * 9000), CorrelationId: cid });
        } else {
          const elapsed = 8 + rnd() * 120 + near * 3000 * rnd();
          emit(inst, t, 'Information', 'Serilog.AspNetCore.RequestLoggingMiddleware', 'HTTP {RequestMethod} {RequestPath} responded {StatusCode} in {Elapsed:0.0000} ms', { RequestMethod: pick(['GET', 'GET', 'POST']), RequestPath: pick(['/api/orders', `/api/orders/${order}`, `/api/orders/${order}/capture`, '/health']), StatusCode: near > 0.5 && rnd() < 0.3 ? 504 : 200, Elapsed: +elapsed.toFixed(4), CorrelationId: cid });
        }
      } else if (inst.kind === 'worker' || inst.kind === 'batch') {
        if (inst.kind === 'batch') {
          emit(inst, t, 'Information', 'Reindex.Job', 'Reindexed {Index} ({Documents} documents) in {ElapsedMs} ms', { Index: pick(['orders', 'customers', 'payments', 'products']), Documents: Math.floor(rnd() * 90000), ElapsedMs: Math.floor(200 + rnd() * 3000) });
        } else if (r < near * 0.3 + 0.002) {
          emit(inst, t, 'Warning', 'Outbox.Dispatcher', 'Outbox dispatch timed out; backlog {Backlog} messages (threshold 1,000)', { Backlog: Math.floor(600 + near * 900 + rnd() * 200) });
        } else if (r < near * 0.35 + 0.003) {
          emit(inst, t, 'Error', 'Outbox.Dispatcher', 'Failed to publish message {MessageId} to {Topic}', { MessageId: guid(), Topic: 'orders.captured' }, HTTP_STACK);
        } else {
          emit(inst, t, 'Information', 'Outbox.Dispatcher', 'Outbox dispatched {Count} messages in {ElapsedMs} ms', { Count: 1 + Math.floor(rnd() * 40), ElapsedMs: Math.floor(5 + rnd() * 90) });
        }
      } else {
        if (r < near * 0.25 + 0.0015) {
          emit(inst, t, 'Warning', 'Yarp.ReverseProxy', 'Upstream {Destination} responded 504 after {ElapsedMs} ms (timeout)', { Destination: pick(['web01:8080', 'web02:8080']), ElapsedMs: 30000 + Math.floor(rnd() * 20) });
        } else if (r < near * 0.29 + 0.0025) {
          emit(inst, t, 'Error', 'Yarp.ReverseProxy', 'No healthy destinations for cluster {Cluster}', { Cluster: 'orders' });
        } else {
          emit(inst, t, 'Information', 'Yarp.ReverseProxy', 'Proxying {Method} {Path} to {Destination}', { Method: pick(['GET', 'POST']), Path: `/api/orders/${88000 + Math.floor(rnd() * 600)}`, Destination: pick(['web01:8080', 'web02:8080']), CorrelationId: cid });
        }
      }
    }

    if (inst.crash) {
      emit(inst, inst.end, 'Fatal', 'Program', 'Host terminated unexpectedly', {}, TIMEOUT_STACK);
    }
    if (inst.stop) {
      emit(inst, inst.end - 600, 'Information', 'Reindex.Job', 'Nightly reindex completed in {Elapsed}', { Elapsed: '00:44:59.4' });
      emit(inst, inst.end, 'Information', 'Microsoft.Hosting.Lifetime', 'Application is shutting down...');
    }
  }

  // Gateway notices the restart.
  const gateway = instances[5];
  if (gateway) emit(gateway, RESTART + 112 * SEC, 'Warning', 'Yarp.ReverseProxy', 'Health probe for {Destination} recovered after {Failures} timeouts', { Destination: 'web02:8080', Failures: 2 });

  // CSV source: a scheduled sync job exporting rows with level and duration columns.
  const sync = { app: 'InventorySync', machine: 'SYNC01', pid: 900, source: 4 };
  for (let t = DEMO_START + 45 * SEC; t < DEMO_END; t += 2 * MIN) {
    const slow = Math.abs(t - INCIDENT) < 8 * MIN;
    emit(sync, t, slow ? 'Warning' : 'Information', 'csv', slow ? 'Sync of {Warehouse} slow: {Items} items in {DurationMs} ms' : 'Synced {Items} items from {Warehouse}', { Warehouse: pick(['AMS-1', 'SIN-2', 'NYC-3']), Items: Math.floor(rnd() * 500), DurationMs: Math.floor((slow ? 9000 : 400) + rnd() * 900) });
  }

  // Runs between lifecycle markers: the shared worker file shows 2208 restarting without a stop (a "no stop seen"
  // run), and WEB01's log begins with the tail of an earlier process (a "stop without start").
  const worker = instances[3];
  if (worker) emit(worker, DEMO_START + 50 * MIN, 'Information', 'Program', 'Application starting — {Application} {Version} ({Framework})', { Application: worker.app, Version: '3.14.1', Framework: 'net9.0' });
  const web01 = instances[0];
  if (web01) emit(web01, DEMO_START, 'Information', 'Microsoft.Hosting.Lifetime', 'Application is shutting down...');

  records.push(...registryRecords(5));

  return {
    sources,
    unrecognized: [
      { path: '/incident-2026-10-14/system/msinfo32.nfo', name: 'msinfo32.nfo', reason: 'No importer for MSInfo exports (plugin)' },
    ],
    records,
  };
}

type RegType = 'REG_SZ' | 'REG_DWORD' | 'REG_QWORD' | 'REG_BINARY' | 'REG_MULTI_SZ' | 'REG_EXPAND_SZ' | 'DELETE';

/** Records a Windows `.reg` export would yield: a key line, then one record per value (no timestamps). */
export function registryRecords(source: number): MockRecord[] {
  const out: MockRecord[] = [];
  let line = 0;
  const key = (hive: string, path: string, type: 'KEY' | 'DELETE' = 'KEY') => {
    out.push({
      id: 0,
      ts: null,
      level: 'Information',
      message: `${hive}\\${path}`,
      template: null,
      exception: null,
      fields: { Hive: hive, Key: path, ValueType: type },
      source,
      line: ++line,
      markers: [],
    });
  };
  const value = (hive: string, path: string, name: string, type: RegType, data: string | number | string[] | null) => {
    out.push({
      id: 0,
      ts: null,
      level: 'Information',
      message: `${hive}\\${path}\\${name} = ${Array.isArray(data) ? data.join(', ') : data}`,
      template: null,
      exception: null,
      fields: { Hive: hive, Key: path, ValueName: name, ValueType: type, Data: data },
      source,
      line: ++line,
      markers: [],
    });
  };
  const H = 'HKEY_LOCAL_MACHINE';
  key(H, 'SOFTWARE\\Contoso\\OrderService');
  value(H, 'SOFTWARE\\Contoso\\OrderService', '(Default)', 'REG_SZ', 'OrderService');
  value(H, 'SOFTWARE\\Contoso\\OrderService', 'InstallPath', 'REG_SZ', 'C:\\Program Files\\Contoso\\OrderService');
  value(H, 'SOFTWARE\\Contoso\\OrderService', 'Version', 'REG_SZ', '1.4.2');
  value(H, 'SOFTWARE\\Contoso\\OrderService', 'TimeoutSeconds', 'REG_DWORD', 30);
  key(H, 'SOFTWARE\\Contoso\\OrderService\\Logging');
  value(H, 'SOFTWARE\\Contoso\\OrderService\\Logging', 'Level', 'REG_SZ', 'Information');
  value(H, 'SOFTWARE\\Contoso\\OrderService\\Logging', 'MaxFileSize', 'REG_QWORD', 104857600);
  value(H, 'SOFTWARE\\Contoso\\OrderService\\Logging', 'Sinks', 'REG_MULTI_SZ', ['File', 'Console']);
  value(H, 'SOFTWARE\\Contoso\\OrderService\\Logging', 'LegacyFlag', 'DELETE', null);
  key(H, 'SOFTWARE\\Contoso\\OrderService\\Obsolete', 'DELETE');
  key(H, 'SYSTEM\\CurrentControlSet\\Services\\OrderService');
  value(H, 'SYSTEM\\CurrentControlSet\\Services\\OrderService', 'ImagePath', 'REG_EXPAND_SZ', '%ProgramFiles%\\Contoso\\OrderService\\svc.exe');
  value(H, 'SYSTEM\\CurrentControlSet\\Services\\OrderService', 'FailureActions', 'REG_BINARY', '80,51,01,00,00,00,00,00');
  value('HKEY_CURRENT_USER', 'Software\\Contoso\\Client', 'ServerUrl', 'REG_SZ', 'https://orders.contoso.test');
  return out;
}

/** A small extra source, so "Open files" does something visible in the browser demo. */
export function generateExtraSource(id: number, path: string, seed: number): { source: MockSource; records: MockRecord[] } {
  const rnd = mulberry32(seed);
  const name = path.split(/[\\/]/).pop() || path;
  const inst = { app: 'Notifications', machine: 'WEB03', pid: 7001 + id, source: id };
  const records: MockRecord[] = [];
  let line = 0;
  const add = (ts: number, level: Level, template: string, props: Record<string, unknown>, exception: string | null = null) => {
    records.push({
      id: 0,
      ts,
      level,
      message: renderTemplate(template, props),
      template,
      exception,
      fields: { ...props, SourceContext: 'Notifications.Sender', Application: inst.app, MachineName: inst.machine, ProcessId: inst.pid, EnvironmentName: 'Production' },
      source: id,
      line: ++line,
      markers: [],
    });
  };
  const start = DEMO_START + 20 * MIN;
  add(start, 'Information', 'Application starting — {Application} {Version} ({Framework})', { Application: inst.app, Version: '1.4.0', Framework: 'net9.0' });
  for (let t = start + 5 * SEC; t < DEMO_END; t += 20 * SEC + rnd() * 40 * SEC) {
    const r = rnd();
    if (r < 0.05) add(t, 'Error', 'SMTP send to {Recipient} failed', { Recipient: 'ops@example.com' }, HTTP_STACK);
    else if (r < 0.12) add(t, 'Warning', 'Retrying notification {NotificationId}', { NotificationId: Math.floor(rnd() * 1e6) });
    else add(t, 'Information', 'Sent {Count} notifications', { Count: 1 + Math.floor(rnd() * 10) });
  }
  return { source: { id, path, name, importer: 'text', skipped: 0 }, records };
}
