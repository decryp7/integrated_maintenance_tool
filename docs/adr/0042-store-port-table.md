# 0042. One port table for the store worker protocol

Status: accepted (2026-10-22)

## Context

`Store` is a supertrait of fourteen port traits. Each new port method was wired three times by hand: a variant in
the `ops!` enum (`imt-store-client` protocol), an arm in the worker's `call` match and a method in `RemoteStore`.
The three lists drifted easily and the order of the enum (the postcard encoding) was only a comment.

## Decision

`imt-store-client/src/port_table.rs` holds **one declarative table**, `store_port_table!`, listing per port trait
every method that crosses to the worker. Three callbacks expand the same table:

- `store_ops_enum!` generates `StoreOp` (`ALL`, `name`);
- `remote_impls!` (client, `remote_ports.rs`) generates every `impl Port for RemoteStore`;
- `worker_dispatch!` (worker, `dispatch.rs`) generates the worker's `match` over `StoreOp`.

Row kinds: `call`, `long` (long timeout) and `analysis` (cancellable, streams `QueryProgress`, takes the thread's
`QueryControl`) are plain rows with one request DTO (or none) and one reply DTO: nothing else to write.
`custom` rows (import sessions, several arguments, state the client remembers, the worker's memory fill) name a
`remote_*` client handler and a `serve_*` worker handler; the compiler fails if either is missing. Members of a
port impl that are not operations (`Query::scan`, `Projects::close`) stay in `remote_extras!`.

The table's types resolve through `protocol::ty`, so the table is written once without paths.

### Wire format

The operation order is the encoding. `WIRE_VERSION` (1) travels in the `Handle` of `OpenStore`; the client refuses a
worker that reports another version with a message to reinstall. A release only appends rows; a test
(`the_wire_format_is_append_only`) pins the existing prefix, so reordering or removing an operation fails it and
requires bumping `WIRE_VERSION` and the snapshot.

## Adding a port

1. The trait method in `imt_core::ports` and its implementation in the store.
2. One row in `store_port_table!`.

A toy port test (`a_new_port_is_one_table_row`) shows that one row yields the operation and the client method; the
worker's exhaustive `match` makes forgetting the dispatch a compile error.

## Consequences

The three hand-written lists are gone (about 400 lines). The trait-per-block layout also documents which port owns
which operation. Plain rows cannot have two arguments: those are `custom` rows.
