//! The client side of the store ports, expanded from the port table ([`crate::store_port_table`], ADR 0039):
//! one `impl Port for RemoteStore` per port. Plain rows are a one-line call; `custom` rows delegate to the
//! `remote_*` methods below, which hold what is not a plain DTO call (import sessions, remembered state).

use super::*;
use crate::protocol::ty::*;
use crate::protocol::AssignParser;
#[cfg(test)]
use crate::store_ops_enum;

/// Expands the table into the client's trait impls.
macro_rules! remote_impls {
    ([$target:ident, $ops:ident]
     $( $trait:ident {
         plain { $( $kind:ident $pop:ident : fn $pm:ident ( $($pa:ident : & $pt:ty)? ) -> $pr:ty ; )* }
         custom { $( $cop:ident : fn $cm:ident ( $($ca:ident : $ct:ty),* ) -> $cr:ty = client $cc:ident, server $cs:ident ; )* }
     } )*
     ;
     wire_only { $( $wop:ident = server $ws:ident ; )* }
    ) => {
        $(
            impl $trait for $target {
                $( remote_impls!(@plain $ops $kind $pop $pm ($($pa : $pt)?) $pr); )*
                $( fn $cm(&self, $($ca: $ct),*) -> StoreResult<$cr> { self.$cc($($ca),*) } )*
                remote_extras!($trait);
            }
        )*
    };
    (@plain $ops:ident $kind:ident $op:ident $m:ident () $r:ty) => {
        fn $m(&self) -> StoreResult<$r> {
            self.$kind($ops::$op, &())
        }
    };
    (@plain $ops:ident $kind:ident $op:ident $m:ident ($a:ident : $t:ty) $r:ty) => {
        fn $m(&self, $a: &$t) -> StoreResult<$r> {
            self.$kind($ops::$op, $a)
        }
    };
}

/// The members of a port impl that are not operations.
macro_rules! remote_extras {
    (Projects) => {
        fn store_dir(&self) -> PathBuf {
            self.dir.clone()
        }
        fn close(&self) {
            self.remote_close()
        }
    };
    (Query) => {
        fn scan(&self, filter: &Filter, visit: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
            self.remote_scan(filter, visit)
        }
    };
    ($other:ident) => {};
}

crate::store_port_table!(remote_impls, [RemoteStore, StoreOp]);

impl RemoteStore {
    /// Closes the store in the worker (the folder stays). Later calls reopen it.
    fn remote_close(&self) {
        let Ok(lease) = self.worker.inner.sup.lease() else { return };
        let handle = lock(&lease.conn.handles).remove(&self.dir);
        if let Some(store) = handle {
            let _ = raw(&lease.conn, &StoreRequest::CloseStore { store }, CALL_TIMEOUT);
        }
    }

    fn remote_export_project(&self, manifest: &ProjectManifest, out: &Path) -> StoreResult<u64> {
        self.long(
            StoreOp::ExportProject,
            &ExportProject {
                manifest: manifest.clone(),
                out: out.to_path_buf(),
            },
        )
    }

    fn remote_open(&self, paths: &[PathBuf]) -> StoreResult<ImportSession> {
        let s = self.call(StoreOp::Open, &Paths { paths: paths.to_vec() })?;
        self.session(s)
    }
    fn remote_reload(&self) -> StoreResult<ImportSession> {
        let s = self.call(StoreOp::Reload, &())?;
        self.session(s)
    }
    fn remote_refresh(&self, changed: &[PathBuf]) -> StoreResult<ImportSession> {
        let s = self.call(StoreOp::Refresh, &Paths { paths: changed.to_vec() })?;
        self.session(s)
    }
    fn remote_remove_source(&self, id: SourceId) -> StoreResult<()> {
        self.call(StoreOp::RemoveSource, &Source { id })
    }
    fn remote_restore(&self, state: &WorkspaceState) -> StoreResult<ImportSession> {
        let s = self.call(StoreOp::Restore, state)?;
        self.session(s)
    }
    fn remote_append_tail(&self, changed: &[PathBuf]) -> StoreResult<TailAppend> {
        self.long(StoreOp::AppendTail, &Paths { paths: changed.to_vec() })
    }
    fn remote_open_at_end(&self, paths: &[PathBuf]) -> StoreResult<ImportSession> {
        let s = self.call(StoreOp::OpenAtEnd, &Paths { paths: paths.to_vec() })?;
        self.session(s)
    }

    fn remote_run_query(&self, req: &QueryRequest, control: &QueryControl) -> StoreResult<QlResult> {
        self.call_controlled(StoreOp::RunQuery, req, control)
    }
    fn remote_check_query(&self, text: &str) -> StoreResult<Option<QlError>> {
        self.call(StoreOp::CheckQuery, &text)
    }

    fn remote_ids_at(&self, from: u64, len: usize) -> StoreResult<Vec<EventId>> {
        self.call(StoreOp::IdsAt, &IdsAt { from, len })
    }
    fn remote_position_of(&self, id: EventId) -> StoreResult<Option<u64>> {
        self.call(StoreOp::PositionOf, &id)
    }

    fn remote_set_profile(&self, profile: Profile) -> StoreResult<ImportSession> {
        let s = self.call(StoreOp::SetProfile, &profile)?;
        self.remember_profile(&profile);
        self.session(s)
    }
    fn remote_set_parsers(&self, setup: &ParserSetup) -> StoreResult<Vec<SourceId>> {
        let stale = self.call(StoreOp::SetParsers, setup)?;
        self.remember_parsers(setup);
        Ok(stale)
    }
    fn remote_reread_parsers(&self) -> StoreResult<ImportSession> {
        let s = self.call(StoreOp::RereadParsers, &())?;
        self.session(s)
    }
    fn remote_assign_parser(&self, source: SourceId, parser: Option<&str>) -> StoreResult<ImportSession> {
        let s = self.call(
            StoreOp::AssignParser,
            &AssignParser {
                source,
                parser: parser.map(str::to_string),
            },
        )?;
        self.session(s)
    }

    /// The store's memory with the worker process's (resident memory, Low memory mode).
    fn remote_store_memory(&self) -> StoreResult<StoreMemory> {
        self.call(StoreOp::StoreMemory, &())
    }
    /// Compaction rewrites segments: a long call.
    fn remote_maintain(&self, action: MaintenanceAction) -> StoreResult<MaintenanceReport> {
        self.long(StoreOp::StoreMaintain, &action)
    }

    /// Batches stream from the worker as Arrow IPC with credits ([`crate::arrow_wire`]); `visit` breaking
    /// cancels the stream.
    pub(crate) fn remote_scan(&self, filter: &Filter, visit: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
        let filter = encode(filter)?;
        let lease = self.worker.inner.sup.lease().map_err(ipc_error)?;
        let result = (|| {
            let store = self.ensure_open(&lease.conn, false)?;
            let mut stream = lease
                .conn
                .child
                .peer()
                .stream_request(&StoreRequest::Scan { store, filter }, protocol::SCAN_CREDIT)
                .map_err(|e| ipc_error(died(&lease.conn, e)))?;
            loop {
                match stream.next(Some(LONG_TIMEOUT)).map_err(|e| ipc_error(died(&lease.conn, e)))? {
                    StreamEvent::Idle => return Err(StoreError::Engine("the store worker stopped sending the scan".into())),
                    StreamEvent::Item(bytes) => {
                        let batch = crate::arrow_wire::decode(&bytes).map_err(|e| StoreError::Engine(e.0))?;
                        if visit(&batch).is_break() {
                            stream.cancel();
                            return Ok(());
                        }
                    }
                    StreamEvent::Done(_) => return Ok(()),
                }
            }
        })();
        if let Err(e) = &result {
            self.worker.report(&lease, e);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A toy port: one table row gives the operation and the client method (the worker's dispatch is an exhaustive
    /// match over the same enum, so it cannot be forgotten either).
    trait Pinger {
        fn ping(&self) -> StoreResult<u8>;
        fn add(&self, n: &u8) -> StoreResult<u8>;
    }

    struct Toy(Mutex<Vec<&'static str>>);

    impl Toy {
        fn call<T: DeserializeOwned>(&self, op: ToyOp, body: &impl Serialize) -> StoreResult<T> {
            self.0.lock().unwrap().push(op.name());
            let n = serde_json::to_value(body).unwrap().as_u64().unwrap_or(0);
            Ok(serde_json::from_value(serde_json::json!(n + 7)).unwrap())
        }
        fn long<T: DeserializeOwned>(&self, op: ToyOp, body: &impl Serialize) -> StoreResult<T> {
            self.call(op, body)
        }
        fn analysis<T: DeserializeOwned>(&self, op: ToyOp, body: &impl Serialize) -> StoreResult<T> {
            self.call(op, body)
        }
    }

    macro_rules! toy_table {
        ($cb:ident, [$($ctx:tt)*]) => {
            $cb! { [$($ctx)*]
                Pinger {
                    plain {
                        call Ping: fn ping() -> u8;
                        long Add: fn add(n: &u8) -> u8;
                    }
                    custom {}
                }
                ;
                wire_only {}
            }
        };
    }

    toy_table!(store_ops_enum, [ToyOp]);
    toy_table!(remote_impls, [Toy, ToyOp]);

    #[test]
    fn a_new_port_is_one_table_row() {
        assert_eq!(ToyOp::ALL, &[ToyOp::Ping, ToyOp::Add]);
        let toy = Toy(Mutex::new(Vec::new()));
        assert_eq!(toy.ping().unwrap(), 7);
        assert_eq!(toy.add(&3).unwrap(), 10);
        assert_eq!(*toy.0.lock().unwrap(), ["Ping", "Add"]);
    }
}
