//! The worker's dispatch of store operations, expanded from the port table ([`imt_store_client::store_port_table`],
//! ADR 0039): a plain row calls the port method with its decoded request; a `custom` row calls the `serve_*`
//! handler below. A new plain port method needs no code here.

use super::*;
use imt_store_client::protocol::ty::*;

/// What a call may use besides its store and body: the caller's cancel probe and the stream for progress items.
pub(super) struct Call<'a> {
    pub probe: Option<CancelProbe>,
    pub items: &'a mut dyn FnMut(Vec<u8>) -> bool,
}

macro_rules! worker_dispatch {
    ([$worker:ident]
     $( $trait:ident {
         plain { $( $kind:ident $pop:ident : fn $pm:ident ( $($pa:ident : & $pt:ty)? ) -> $pr:ty ; )* }
         custom { $( $cop:ident : fn $cm:ident ( $($ca:ident : $ct:ty),* ) -> $cr:ty = client $cc:ident, server $cs:ident ; )* }
     } )*
     ;
     wire_only { $( $wop:ident = server $ws:ident ; )* }
    ) => {
        impl $worker {
            /// Runs one operation on the open store `s`; the reply is its DTO as JSON.
            pub(super) fn run_op(&self, s: &DuckStore, op: StoreOp, b: serde_json::Value, cx: Call<'_>) -> StoreResult<serde_json::Value> {
                match op {
                    $(
                        $( StoreOp::$pop => worker_dispatch!(@plain $trait $kind $pm ($($pt)?) s b cx), )*
                        $( StoreOp::$cop => self.$cs(s, b, cx), )*
                    )*
                    $( StoreOp::$wop => self.$ws(s, b), )*
                }
            }
        }
    };
    (@plain $trait:ident call $m:ident ($($t:ty)?) $s:ident $b:ident $cx:ident) => {
        reply(&$trait::$m($s $(, &body::<$t>($b)?)?)?)
    };
    (@plain $trait:ident long $m:ident ($($t:ty)?) $s:ident $b:ident $cx:ident) => {
        reply(&$trait::$m($s $(, &body::<$t>($b)?)?)?)
    };
    (@plain $trait:ident analysis $m:ident ($($t:ty)?) $s:ident $b:ident $cx:ident) => {
        reply(&controlled($cx.probe, $cx.items, || $trait::$m($s $(, &body::<$t>($b)?)?))?)
    };
}

imt_store_client::store_port_table!(worker_dispatch, [Worker]);

impl Worker {
    fn serve_open(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        let p: Paths = body(b)?;
        reply(&self.session(Ingest::open(s, &p.paths)?))
    }
    fn serve_reload(&self, s: &DuckStore, _: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        reply(&self.session(Ingest::reload(s)?))
    }
    fn serve_refresh(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        let p: Paths = body(b)?;
        reply(&self.session(Ingest::refresh(s, &p.paths)?))
    }
    fn serve_remove_source(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        reply(&Ingest::remove_source(s, body::<Source>(b)?.id)?)
    }
    fn serve_restore(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        let state: WorkspaceState = body(b)?;
        reply(&self.session(Ingest::restore(s, &state)?))
    }
    fn serve_append_tail(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        reply(&Ingest::append_tail(s, &body::<Paths>(b)?.paths)?)
    }
    fn serve_open_at_end(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        let p: Paths = body(b)?;
        reply(&self.session(Tail::open_at_end(s, &p.paths)?))
    }

    fn serve_run_query(&self, s: &DuckStore, b: serde_json::Value, cx: Call<'_>) -> StoreResult<serde_json::Value> {
        let req = body(b)?;
        let control = QueryControl::new();
        let _watch = cx.probe.map(|p| Watch::start(p, control.clone()));
        reply(&with_progress(&control, cx.items, || Query::run_query_with(s, &req, &control))?)
    }
    fn serve_check_query(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        reply(&Query::check_query(s, &body::<String>(b)?)?)
    }

    fn serve_ids_at(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        let r: IdsAt = body(b)?;
        reply(&Positions::ids_at(s, r.from, r.len)?)
    }
    fn serve_position_of(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        reply(&Positions::position_of(s, body(b)?)?)
    }

    fn serve_set_profile(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        reply(&self.session(Profiles::set_profile(s, body::<Profile>(b)?)?))
    }
    fn serve_set_parsers(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        reply(&Profiles::set_parsers(s, &body(b)?)?)
    }
    fn serve_reread_parsers(&self, s: &DuckStore, _: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        reply(&self.session(Profiles::reread_parsers(s)?))
    }
    fn serve_assign_parser(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        let r: protocol::AssignParser = body(b)?;
        reply(&self.session(Profiles::assign_parser(s, r.source, r.parser.as_deref())?))
    }

    fn serve_export_project(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        let r: ExportProject = body(b)?;
        reply(&Projects::export_project(s, &r.manifest, &r.out)?)
    }

    /// The store's memory with this worker process filled in.
    fn serve_store_memory(&self, s: &DuckStore, _: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        let mut m = Inspect::store_memory(s)?;
        m.worker = Some(imt_core::ports::WorkerMemory {
            pid: std::process::id(),
            rss_bytes: imt_ipc::process::current_rss_bytes(),
            low_memory: self.mode == MemoryMode::Low,
        });
        reply(&m)
    }
    fn serve_maintain(&self, s: &DuckStore, b: serde_json::Value, _: Call<'_>) -> StoreResult<serde_json::Value> {
        reply(&Maintain::maintain(s, body(b)?)?)
    }

    // The import session's handle (no port method).
    fn serve_session_progress(&self, s: &DuckStore, _: serde_json::Value) -> StoreResult<serde_json::Value> {
        reply(&s.progress().snapshot())
    }
    fn serve_session_cancel(&self, s: &DuckStore, _: serde_json::Value) -> StoreResult<serde_json::Value> {
        s.progress().cancel();
        reply(&())
    }
    fn serve_session_wait(&self, _: &DuckStore, b: serde_json::Value) -> StoreResult<serde_json::Value> {
        let id: Session = body(b)?;
        let session = lock(&self.sessions)
            .remove(&id.session)
            .ok_or_else(|| StoreError::NotFound(format!("import {} (already waited for?)", id.session)))?;
        reply(&session.wait()?)
    }
}
