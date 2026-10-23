import { P, save, link, hub, bottom, Y0 } from './common.mjs';

const d = P({ title: 'Store protocol — one port table', kind: 'UML · Component diagram', w: 1536,
  desc: '`store_port_table!` lists every port method that crosses to the store worker once; three macros expand it (ADR 0042). The wire is version 2 since the import-window row (ADR 0047).' });
const W = 240, G = 96, x = (i) => 48 + i * (W + G);
const gh = 40 + 3 * 112 + 2 * 24 + 24;
d.group('b', 48, Y0, 3 * W + 2 * G + 48, gh, 'Build time: one table, three expansions');
const ry = (i) => Y0 + 40 + i * (112 + 24);
d.node('p1', 72, ry(0), W, 3 * 112 + 48, 'core', { name: 'debrief_core::ports', tech: 'Rust traits', desc: 'Fourteen traits combined as `Store`: `Ingest`, `Query`, `Positions`, `Analysis`, `Profiles`, `Diagnostics`, `Search`, `Projects`, `Inspect`, `Maintain`, `Console`, `Redaction`, `Skew`, `Tail`.' });
d.node('p2', 72 + W + G, ry(0), W, 3 * 112 + 48, 'accent', { name: 'store_port_table!', tech: 'port_table.rs, the one declarative table', desc: 'Per port, one row per method. `call`, `long` and `analysis` rows take DTOs only; `custom` rows name a `remote_*` client handler and a `serve_*` worker handler. The row order is the wire format.' });
const ex = 72 + 2 * (W + G);
d.node('p3', ex, ry(0), W, 112, 'accent', { name: 'store_ops_enum!', tech: '`StoreOp` in protocol.rs', desc: 'The operation enum; append-only, pinned by a test.' });
d.node('p4', ex, ry(1), W, 112, 'accent', { name: 'remote_impls!', tech: '`impl Port for RemoteStore`', desc: 'Client side: one request frame per call.' });
d.node('p5', ex, ry(2), W, 112, 'accent', { name: 'worker_dispatch!', tech: 'dispatch.rs in debrief-store-worker', desc: 'One exhaustive `match` over `StoreOp`: a forgotten arm does not compile.' });
link(d, 'p1', 'R', 'p2', 'L', { label: 'Rows' });
hub(d, 'p2', 'R', 'p3', 'L', { label: 'Expands' });
hub(d, 'p2', 'R', 'p4', 'L', { label: 'Expands' });
hub(d, 'p2', 'R', 'p5', 'L', { label: 'Expands' });
// run time
const y2 = Y0 + gh + 56, W2 = 208, G2 = 88, x2 = (i) => 48 + i * (W2 + G2);
d.group('r', 48, y2, 5 * W2 + 4 * G2 + 48, 40 + 152 + 24, 'Run time: one call from the shell to the store');
const rn = (id, i, role, o) => d.node(id, 72 + i * (W2 + G2) - 0, y2 + 40, W2, 152, role, o);
const rx = (i) => 72 + i * (W2 + G2);
d.node('c1', rx(0), y2 + 40, W2, 152, 'accent', { name: 'Tauri shell, `debrief`', tech: 'Callers', desc: 'Code against the `Store` ports only; never a concrete store.' });
d.node('c2', rx(1), y2 + 40, W2, 152, 'accent', { name: 'RemoteStore', tech: 'debrief-store-client', desc: 'Sends the request as a frame; `Query::scan` and `Projects::close` stay in `remote_extras!`.' });
d.node('c3', rx(2), y2 + 40, W2, 152, 'process', { name: 'debrief-ipc frames', tech: 'Length-prefixed postcard', desc: '`OpenStore` carries `WIRE_VERSION`; the client refuses a worker with another version.' });
d.node('c4', rx(3), y2 + 40, W2, 152, 'accent', { name: 'Worker dispatch', tech: 'debrief-store-worker', desc: 'Dispatches the operation to the store of the calling window.' });
d.node('c5', rx(4), y2 + 40, W2, 152, 'accent', { name: 'DuckStore', tech: 'debrief-store-duck', desc: 'Implements every port over Parquet segments, the DuckDB catalog and the Tantivy index.' });
link(d, 'c1', 'R', 'c2', 'L', { label: 'Calls' });
link(d, 'c2', 'R', 'c3', 'L', { label: 'Sends' });
link(d, 'c3', 'R', 'c4', 'L', { label: 'Delivers' });
link(d, 'c4', 'R', 'c5', 'L', { label: 'Calls' });
d.node('nt', 48 + 3 * W + 2 * G + 48 + 56, Y0, 1488 - (48 + 3 * W + 2 * G + 48 + 56), null, 'process', { name: 'Adding a port method', desc: 'The trait method in `debrief_core::ports`, its implementation in the store, and one row in `store_port_table!`, appended at the end. Reordering or removing a row bumps `WIRE_VERSION` and the snapshot test.' });
d.legend([{ kind: 'core', label: 'Port traits' }, { kind: 'accent', label: 'Debrief component' }, { kind: 'process', label: 'Transport · note' }, { kind: 'line', label: 'Uses' }]);
save('uml-component-store-protocol', d, y2 + 40 + 152 + 24);
