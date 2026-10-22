use imt_core::exceptions::{fingerprint, group, parse_exception, ExceptionInfo};
use imt_core::model::{Level, Record};
use imt_core::patterns::{compare, mine, tokenize, Change, MinerConfig, PatternMiner, TemplateSource};

fn rec(id: u32, msg: &str) -> Record {
    let mut r = Record::new(id);
    r.id = id;
    r.message = msg.to_string();
    r
}

fn exc_rec(id: u32, ts: Option<i64>, text: &str) -> Record {
    let mut r = rec(id, "boom");
    r.ts = ts;
    r.exception = Some(text.to_string());
    r
}

// ---------------------------------------------------------------------------
// Stack traces
// ---------------------------------------------------------------------------

const DOTNET: &str = "System.InvalidOperationException: Sequence contains no elements
   at System.Linq.ThrowHelper.ThrowNoElementsException()
   at System.Linq.Enumerable.First[TSource](IEnumerable`1 source)
   at Acme.Orders.OrderService.<>c__DisplayClass4_0.<LoadAsync>b__0(Order o) in C:\\src\\Acme\\OrderService.cs:line 88
   at Acme.Orders.OrderService.Load(Int32 id) in C:\\src\\Acme\\OrderService.cs:line 42
   at Acme.Program.Main(String[] args)";

const DOTNET_INNER: &str = "System.AggregateException: One or more errors occurred. (Disk full)
 ---> System.IO.IOException: Disk full
   at System.IO.FileStream.Write(Byte[] buffer, Int32 offset, Int32 count)
   at Acme.Store.Save(String path) in /src/Store.cs:line 10
 ---> System.Exception: deepest
   at Acme.Deep.Thing()
   --- End of inner exception stack trace ---
   at Acme.Store.SaveAll() in /src/Store.cs:line 20
   --- End of inner exception stack trace ---
   at Acme.Runner.Run() in /src/Runner.cs:line 5";

const DOTNET_ASYNC_A: &str = "System.NullReferenceException: Object reference not set to an instance of an object.
   at Acme.Svc.<ProcessAsync>d__7.MoveNext() in /src/Svc.cs:line 31
--- End of stack trace from previous location ---
   at System.Runtime.CompilerServices.TaskAwaiter.ThrowForNonSuccess(Task task)
   at System.Runtime.CompilerServices.TaskAwaiter.HandleNonSuccessAndDebuggerNotification(Task task)
   at Acme.Svc.RunAsync() in /src/Svc.cs:line 12";

const DOTNET_ASYNC_B: &str = "System.NullReferenceException: Object reference not set to an instance of an object.
   at Acme.Svc.<ProcessAsync>d__9.MoveNext() in /src/Svc.cs:line 77
   at Acme.Svc.RunAsync() in /src/Svc.cs:line 99";

const JAVA: &str = "java.lang.IllegalStateException: Cannot start
\tat com.acme.Service.start(Service.java:57)
\tat com.acme.Service$1.run(Service.java:30)
\tat com.acme.Main.lambda$main$0(Main.java:12)
\tat java.base/java.lang.Thread.run(Thread.java:834)
\tat com.acme.Native.call(Native Method)
\tat com.acme.Unk.call(Unknown Source)
Caused by: java.io.IOException: Broken pipe
\tat java.base/sun.nio.ch.FileDispatcherImpl.write0(Native Method)
\tat com.acme.Io.write(Io.java:9)
\t... 3 more
\tSuppressed: java.lang.Exception: ignored
\t\tat com.acme.Cleanup.close(Cleanup.java:1)
Caused by: java.net.SocketException: Reset
\tat com.acme.Net.read(Net.java:3)
\tat com.acme.Svc$$Lambda$14/0x0000000800c03000.run(Unknown Source)
\tat com.acme.Svc$$EnhancerBySpringCGLIB$$abc123.proxy(<generated>)";

const PYTHON: &str = "Traceback (most recent call last):
  File \"/app/main.py\", line 10, in <module>
    run()
  File \"/app/runner.py\", line 22, in run
    x = compute(1)
  File \"/app/calc.py\", line 5, in compute
    return 1 / 0
ZeroDivisionError: division by zero";

const PYTHON_CHAINED: &str = "Traceback (most recent call last):
  File \"/app/db.py\", line 3, in connect
    raise KeyError('host')
KeyError: 'host'

The above exception was the direct cause of the following exception:

Traceback (most recent call last):
  File \"/app/main.py\", line 8, in start
    connect()
RuntimeError: cannot connect";

const JS: &str = "TypeError: Cannot read properties of undefined (reading 'id')
    at Object.getUser (/srv/app/users.js:41:17)
    at async Promise.all (index 0)
    at new Handler (C:\\srv\\app\\handler.js:7:3)
    at /srv/app/server.js:120:9
    at processTicks (node:internal/process/task_queues:95:5)
    at native (native)
    at anon (<anonymous>)
    at file (/x/y.js:5)";

fn funcs(i: &ExceptionInfo) -> Vec<&str> {
    i.frames.iter().map(|f| f.function.as_str()).collect()
}

#[test]
fn parses_dotnet() {
    let e = parse_exception(DOTNET).unwrap();
    assert_eq!(e.type_name, "System.InvalidOperationException");
    assert_eq!(e.message, "Sequence contains no elements");
    assert_eq!(e.frames.len(), 5);
    assert_eq!(e.frames[0].function, "System.Linq.ThrowHelper.ThrowNoElementsException");
    assert_eq!(e.frames[0].file, None);
    assert_eq!(e.frames[1].function, "System.Linq.Enumerable.First[TSource]");
    assert_eq!(e.frames[2].file.as_deref(), Some("C:\\src\\Acme\\OrderService.cs"));
    assert_eq!(e.frames[2].line, Some(88));
    assert!(e.inner.is_none());
}

#[test]
fn parses_dotnet_inner_exceptions() {
    let e = parse_exception(DOTNET_INNER).unwrap();
    assert_eq!(e.type_name, "System.AggregateException");
    assert_eq!(e.message, "One or more errors occurred. (Disk full)");
    // The outer exception's own frames come after the end markers.
    assert_eq!(funcs(&e), ["Acme.Runner.Run"]);
    let inner = e.inner.as_ref().unwrap();
    assert_eq!(inner.type_name, "System.IO.IOException");
    assert_eq!(
        funcs(inner),
        ["System.IO.FileStream.Write", "Acme.Store.Save", "Acme.Store.SaveAll"]
    );
    let deepest = inner.inner.as_ref().unwrap();
    assert_eq!(deepest.message, "deepest");
    assert_eq!(funcs(deepest), ["Acme.Deep.Thing"]);
    assert_eq!(e.root().type_name, "System.Exception");
}

#[test]
fn parses_java_with_causes() {
    let e = parse_exception(JAVA).unwrap();
    assert_eq!(e.type_name, "java.lang.IllegalStateException");
    assert_eq!(e.message, "Cannot start");
    assert_eq!(e.frames[0].function, "com.acme.Service.start");
    assert_eq!(e.frames[0].file.as_deref(), Some("Service.java"));
    assert_eq!(e.frames[0].line, Some(57));
    // Module prefix stripped.
    assert_eq!(e.frames[3].function, "java.lang.Thread.run");
    assert_eq!(e.frames[4].file, None);
    let cause = e.inner.as_ref().unwrap();
    assert_eq!(cause.type_name, "java.io.IOException");
    // The suppressed exception's frames do not leak into the cause.
    assert_eq!(cause.frames.len(), 2);
    let root = cause.inner.as_ref().unwrap();
    assert_eq!(root.type_name, "java.net.SocketException");
    assert_eq!(root.frames.len(), 3);
    assert!(root.inner.is_none());
}

#[test]
fn parses_python_top_frame_first() {
    let e = parse_exception(PYTHON).unwrap();
    assert_eq!(e.type_name, "ZeroDivisionError");
    assert_eq!(e.message, "division by zero");
    assert_eq!(funcs(&e), ["compute", "run", "<module>"]);
    assert_eq!(e.frames[0].file.as_deref(), Some("/app/calc.py"));
    assert_eq!(e.frames[0].line, Some(5));
}

#[test]
fn parses_python_chained_and_indented() {
    let e = parse_exception(PYTHON_CHAINED).unwrap();
    assert_eq!(e.type_name, "RuntimeError");
    assert_eq!(e.inner.as_ref().unwrap().type_name, "KeyError");
    assert_eq!(e.inner.as_ref().unwrap().message, "'host'");

    let indented: String = PYTHON.lines().map(|l| format!("    {l}\n")).collect();
    assert_eq!(parse_exception(&indented).unwrap().type_name, "ZeroDivisionError");

    // Frames without the "Traceback" banner, two exceptions back to back.
    let two = "  File \"a.py\", line 1, in f\nValueError: bad\n  File \"b.py\", line 2\nOSError";
    let e = parse_exception(two).unwrap();
    assert_eq!(e.type_name, "OSError");
    assert_eq!(e.frames[0].function, "<module>");
    assert_eq!(e.inner.as_ref().unwrap().type_name, "ValueError");

    // A traceback that never reaches its header line is not an exception.
    assert!(parse_exception("Traceback (most recent call last):\n  File \"a.py\", line 1, in f").is_none());
    // A banner with no frames and then a new banner.
    let e = parse_exception(
        "Traceback (most recent call last):\nTraceback (most recent call last):\n  File \"a.py\", line 1, in f\nKeyError: 'k'",
    )
    .unwrap();
    assert_eq!(e.type_name, "KeyError");
}

#[test]
fn parses_javascript() {
    let e = parse_exception(JS).unwrap();
    assert_eq!(e.type_name, "TypeError");
    assert_eq!(e.message, "Cannot read properties of undefined (reading 'id')");
    assert_eq!(e.frames[0].function, "Object.getUser");
    assert_eq!(e.frames[0].file.as_deref(), Some("/srv/app/users.js"));
    assert_eq!(e.frames[0].line, Some(41));
    assert_eq!(e.frames[1].function, "async Promise.all");
    assert_eq!(e.frames[1].file.as_deref(), Some("index 0"));
    assert_eq!(e.frames[1].line, None);
    assert_eq!(e.frames[2].file.as_deref(), Some("C:\\srv\\app\\handler.js"));
    assert_eq!(e.frames[3].function, "<anonymous>");
    assert_eq!(e.frames[3].line, Some(120));
    assert_eq!(e.frames[5].file.as_deref(), Some("native"));
    assert_eq!(e.frames[7].line, Some(5));
    assert_eq!(e.frames[7].file.as_deref(), Some("/x/y.js"));
}

#[test]
fn parse_edge_cases() {
    assert!(parse_exception("").is_none());
    assert!(parse_exception("   \n  \n").is_none());
    assert!(parse_exception("just a plain sentence about nothing").is_none());
    assert!(parse_exception("Boom").is_none());
    // Header only, with a recognisable exception type.
    let e = parse_exception("System.TimeoutException").unwrap();
    assert_eq!(e.type_name, "System.TimeoutException");
    assert_eq!(e.message, "");
    let e = parse_exception("MyError: nope").unwrap();
    assert_eq!(e.message, "nope");
    // Frames with no header line.
    let e = parse_exception("   at Foo.Bar(String s)\n   at Foo.Baz()").unwrap();
    assert_eq!(e.type_name, "Unknown");
    assert_eq!(funcs(&e), ["Foo.Bar", "Foo.Baz"]);
    // Multi-line message, windows newlines, type without message, non-type prefix.
    let e = parse_exception("Acme.CustomException: line one\r\nline two\r\n   at A.B()\r\n").unwrap();
    assert_eq!(e.message, "line one\nline two");
    assert_eq!(e.frames.len(), 1);
    let e = parse_exception("Some message: with colon\n   at A.B()").unwrap();
    assert_eq!(e.type_name, "Unknown");
    assert_eq!(e.message, "Some message: with colon");
    let e = parse_exception("Foo::Bar happened\n   at A.B()").unwrap();
    assert_eq!(e.type_name, "Unknown");
    // A blank line ends the message block; stray text between frames is ignored.
    let e = parse_exception("X.Error: m\n\nnoise\n   at A.B()").unwrap();
    assert_eq!(e.message, "m");
    assert_eq!(e.frames.len(), 1);
    // Unrecognised "at" lines are not frames.
    assert!(parse_exception("at the end of the day").is_none());
    assert!(parse_exception("   at (anonymous)").is_none());
    // End marker at top level is ignored.
    let e = parse_exception("E.Error: m\n   --- End of inner exception stack trace ---\n   at A.B()").unwrap();
    assert_eq!(e.frames.len(), 1);
    // A dotnet frame with a file but no line number.
    let e = parse_exception("E.Error: m\n   at A.B() in /x/y.cs").unwrap();
    assert_eq!(e.frames[0].line, None);
    assert_eq!(e.frames[0].file.as_deref(), Some("/x/y.cs"));
    // `at Foo (file:line)` without a column keeps the line.
    let e = parse_exception("E.Error: m\n    at foo (/a.js:12)").unwrap();
    assert_eq!(e.frames[0].line, Some(12));
    // Empty parens in a JS frame.
    let e = parse_exception("E.Error: m\n    at foo ()").unwrap();
    assert_eq!(e.frames[0].file, None);
}

#[test]
fn deep_nesting_is_bounded() {
    let mut s = String::from("A.Exception: top\n");
    for _ in 0..5000 {
        s.push_str(" ---> B.Exception: nested\n   at X.y()\n");
    }
    let e = parse_exception(&s).unwrap();
    let mut depth = 0;
    let mut cur = &e;
    while let Some(i) = &cur.inner {
        depth += 1;
        cur = i;
    }
    assert!(depth <= 16);

    let mut s = String::from("A.Exception: top\n");
    for _ in 0..5000 {
        s.push_str("Caused by: B.Exception: nested\n");
    }
    assert!(parse_exception(&s).is_some());

    let mut s = String::from("A.Exception: top\n");
    for i in 0..1000 {
        s.push_str(&format!("   at X.y{i}()\n"));
    }
    assert_eq!(parse_exception(&s).unwrap().frames.len(), 256);
}

// ---------------------------------------------------------------------------
// Fingerprints
// ---------------------------------------------------------------------------

#[test]
fn fingerprint_ignores_lines_and_instances() {
    let a = parse_exception(DOTNET_ASYNC_A).unwrap();
    let b = parse_exception(DOTNET_ASYNC_B).unwrap();
    let fa = fingerprint(&a, 5);
    assert_eq!(fa.len(), 16);
    assert!(fa.bytes().all(|c| c.is_ascii_hexdigit()));
    // Different line numbers, state machine ids and message instances, same bug.
    assert_eq!(fa, fingerprint(&b, 5));

    let mut c = b.clone();
    c.type_name = "System.ArgumentException".into();
    assert_ne!(fa, fingerprint(&c, 5));

    // Only the top frame differs, but beyond the window of 1.
    let mut d = b.clone();
    d.frames[1].function = "Acme.Other".into();
    assert_eq!(fingerprint(&b, 1), fingerprint(&d, 1));
    assert_ne!(fingerprint(&b, 5), fingerprint(&d, 5));
    assert_eq!(fingerprint(&b, 0), fingerprint(&d, 0));
}

#[test]
fn fingerprint_normalises_function_names() {
    let fp = |fns: &[&str]| {
        let mut e = ExceptionInfo {
            type_name: "System.Foo`1".into(),
            message: String::new(),
            frames: fns
                .iter()
                .map(|f| imt_core::exceptions::Frame {
                    function: f.to_string(),
                    file: None,
                    line: None,
                })
                .collect(),
            inner: None,
        };
        let a = fingerprint(&e, 5);
        e.type_name = "System.Foo".into();
        assert_eq!(a, fingerprint(&e, 5), "type arity is ignored");
        a
    };
    let same = |a: &[&str], b: &[&str]| assert_eq!(fp(a), fp(b), "{a:?} vs {b:?}");
    same(&["A.B`2.Run"], &["A.B.Run"]);
    same(&["A.Run[System.Int32]"], &["A.Run"]);
    same(&["A.B.<>c__DisplayClass1_0.<Go>b__0"], &["A.B.Go"]);
    same(&["A.B.<>c.<Go>b__2_1"], &["A.B.Go"]);
    same(&["A.B.<Go>d__4.MoveNext"], &["A.B.Go"]);
    same(&["A.B.<Go>g__Local|0_0"], &["A.B.Go"]);
    same(&["A.B.b__0"], &["A.B"]);
    same(&["A.B.lambda_method123"], &["A.B"]);
    same(&["async A.B.Go"], &["A.B.Go"]);
    same(&["new A.B"], &["A.B"]);
    same(&["A.Go.Go"], &["A.Go"]);
    same(&["a.B$1.run"], &["a.B.run"]);
    same(&["a.B.lambda$main$0"], &["a.B.main"]);
    same(&["a.Svc$$Lambda$14/0x0000000800c03000.run"], &["a.Svc.run"]);
    same(&["a.Svc$$EnhancerBySpringCGLIB$$abc123.proxy"], &["a.Svc.proxy"]);
    // MoveNext is only dropped after a state machine.
    assert_ne!(fp(&["A.Enumerator.MoveNext"]), fp(&["A.Enumerator"]));
    // Framework plumbing does not count, unless that is all there is; empty names are skipped.
    same(&["System.Runtime.CompilerServices.TaskAwaiter.ThrowForNonSuccess", "A.B"], &["A.B"]);
    assert_ne!(
        fp(&["System.Runtime.CompilerServices.X"]),
        fp(&["System.Runtime.CompilerServices.Y"])
    );
    same(&["", "A.B"], &["A.B"]);
    same(&["<>c", "A.B"], &["A.B"]);
}

#[test]
fn fingerprint_uses_root_cause() {
    let wrap = |outer: &str| {
        let mut e = parse_exception(DOTNET_INNER).unwrap();
        e.type_name = outer.to_string();
        fingerprint(&e, 5)
    };
    assert_eq!(wrap("A.Wrapper"), wrap("A.Wrapper"));
    assert_ne!(wrap("A.Wrapper"), wrap("B.Wrapper"));
    let mut shallow = parse_exception(DOTNET_INNER).unwrap();
    shallow.inner.as_mut().unwrap().inner.as_mut().unwrap().frames.clear();
    assert_ne!(fingerprint(&shallow, 5), fingerprint(&parse_exception(DOTNET_INNER).unwrap(), 5));
}

// ---------------------------------------------------------------------------
// Grouping
// ---------------------------------------------------------------------------

#[test]
fn groups_by_fingerprint() {
    let records = [
        exc_rec(1, Some(300), DOTNET_ASYNC_A),
        rec(2, "no exception"),
        exc_rec(3, Some(100), DOTNET_ASYNC_B),
        exc_rec(4, None, DOTNET_ASYNC_B),
        exc_rec(5, Some(200), PYTHON),
        exc_rec(6, Some(50), "something went wrong 17 times"),
        exc_rec(7, Some(60), "\n  something went wrong 99 times\nmore"),
        exc_rec(8, None, "   \n"),
    ];
    let keys = ["a.log", "b.log", "a.log", "a.log", "b.log", "x", "x", "y"];
    let groups = group(records.iter().zip(keys.iter().copied()));
    assert_eq!(groups.len(), 4);
    let g = &groups[0];
    assert_eq!(g.count, 3);
    assert_eq!(g.type_name, "System.NullReferenceException");
    assert_eq!(g.message, "Object reference not set to an instance of an object.");
    assert_eq!(g.top_frame.as_deref(), Some("Acme.Svc.<ProcessAsync>d__7.MoveNext"));
    assert_eq!(g.instances, vec![("a.log".to_string(), 3)]);
    assert_eq!(g.first_ts, Some(100));
    assert_eq!(g.last_ts, Some(300));
    assert_eq!(g.sample_ids, vec![1, 3, 4]);

    let unknown = groups.iter().find(|g| g.type_name == "Unknown" && g.count == 2).unwrap();
    assert_eq!(unknown.message, "something went wrong 17 times");
    assert_eq!(unknown.top_frame, None);
    assert_eq!(unknown.first_ts, Some(50));
    assert!(groups
        .iter()
        .any(|g| g.type_name == "ZeroDivisionError" && g.top_frame.as_deref() == Some("compute")));
    let empty = groups
        .iter()
        .find(|g| g.count == 1 && g.type_name == "Unknown" && g.message.is_empty())
        .unwrap();
    assert_eq!(empty.sample_ids, vec![8]);

    let json = serde_json::to_value(&groups[0]).unwrap();
    assert!(json.get("topFrame").is_some() && json.get("sampleIds").is_some() && json.get("typeName").is_some());
}

#[test]
fn group_limits_samples_and_orders_instances() {
    let records: Vec<Record> = (0..30).map(|i| exc_rec(i, Some(i as i64), PYTHON)).collect();
    let keys: Vec<String> = (0..30).map(|i| format!("host{}", if i % 3 == 0 { "b" } else { "a" })).collect();
    let groups = group(records.iter().zip(keys.iter().map(String::as_str)));
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].sample_ids.len(), 10);
    assert_eq!(groups[0].instances, vec![("hosta".to_string(), 20), ("hostb".to_string(), 10)]);
    assert!(group(std::iter::empty::<(&imt_core::model::Record, &str)>()).is_empty());
    // Equal counts order by fingerprint deterministically; instances tie-break by name.
    let rs = [exc_rec(1, None, PYTHON), exc_rec(2, None, JS)];
    let g = group(rs.iter().zip(["z", "z"]));
    assert!(g[0].fingerprint < g[1].fingerprint);
    let rs = [exc_rec(1, None, PYTHON), exc_rec(2, None, PYTHON)];
    let g = group(rs.iter().zip(["z", "a"]));
    assert_eq!(g[0].instances[0].0, "a");
}

// ---------------------------------------------------------------------------
// Drain
// ---------------------------------------------------------------------------

#[test]
fn drain_collapses_variants() {
    let records: Vec<Record> = (0..20)
        .map(|i| rec(i, &format!("Connected to 10.0.{}.{} in {} ms", i, i + 1, 20 + i * 3)))
        .collect();
    let patterns = mine(&records, &MinerConfig::default());
    assert_eq!(patterns.len(), 1);
    assert_eq!(patterns[0].template, "Connected to <*> in <*> ms");
    assert_eq!(patterns[0].count, 20);
    assert_eq!(patterns[0].source, TemplateSource::Mined);
    assert_eq!(patterns[0].sample_ids, vec![0, 1, 2, 3, 4]);
}

#[test]
fn drain_generalises_words_and_separates_shapes() {
    let mut recs = Vec::new();
    for (i, who) in ["alice", "bob", "carol", "dave", "erin"].iter().enumerate() {
        recs.push(rec(i as u32, &format!("User {who} logged in from host")));
    }
    recs.push(rec(10, "Cache miss for key"));
    recs.push(rec(11, "Cache miss for key"));
    recs.push(rec(12, "totally different shape here with more tokens"));
    // One routing token, so the user name is not part of the tree path.
    let cfg = MinerConfig {
        depth: 3,
        ..MinerConfig::default()
    };
    let p = mine(&recs, &cfg);
    assert_eq!(p.len(), 3);
    assert_eq!(p[0].template, "User <*> logged in from host");
    assert_eq!(p[0].count, 5);
    assert_eq!(p[1].template, "Cache miss for key");
    // A wildcarded template keeps absorbing lines.
    let mut m = PatternMiner::new(cfg);
    let a = m.add(&rec(1, "User alice logged in"));
    let b = m.add(&rec(2, "User bob logged in"));
    let c = m.add(&rec(3, "User carol logged in"));
    assert!(a == b && b == c);
    assert_eq!(m.clusters()[0].template, "User <*> logged in");
}

#[test]
fn drain_threshold_and_depth_config() {
    let lines = ["alpha beta gamma delta", "alpha beta gamma omega", "alpha zeta eta theta"];
    let recs: Vec<Record> = lines.iter().enumerate().map(|(i, l)| rec(i as u32, l)).collect();
    // Strict threshold keeps them apart even though the prefix routes them together.
    let strict = MinerConfig {
        similarity_threshold: 0.9,
        depth: 3,
        max_children: 100,
    };
    assert_eq!(mine(&recs, &strict).len(), 3);
    // Loose threshold merges everything that shares a leaf.
    let loose = MinerConfig {
        similarity_threshold: 0.2,
        depth: 2,
        max_children: 100,
    };
    let p = mine(&recs, &loose);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].template, "alpha <*> <*> <*>");
}

#[test]
fn drain_max_children_overflow_shares_wildcard() {
    let cfg = MinerConfig {
        similarity_threshold: 0.5,
        depth: 4,
        max_children: 2,
    };
    let recs: Vec<Record> = ["one x", "two x", "three x", "four x", "five x"]
        .iter()
        .enumerate()
        .map(|(i, l)| rec(i as u32, l))
        .collect();
    let p = mine(&recs, &cfg);
    // Overflowing first tokens share the wildcard branch and merge into one template there.
    assert_eq!(p.len(), 3, "{p:?}");
    assert_eq!(p[0].template, "<*> x");
    assert_eq!(p.iter().map(|p| p.count).sum::<usize>(), 5);
    // max_children of 0 behaves like 1 rather than panicking.
    let zero = MinerConfig {
        max_children: 0,
        ..MinerConfig::default()
    };
    assert_eq!(mine(&recs, &zero).iter().map(|p| p.count).sum::<usize>(), 5);
}

#[test]
fn serilog_templates_group_exactly() {
    let mut recs = Vec::new();
    for i in 0..4u32 {
        let mut r = rec(i, &format!("Handled request {i} for user u{i}"));
        r.template = Some("Handled request {RequestId} for user {User}".into());
        r.level = if i == 3 { Level::Error } else { Level::Information };
        r.ts = Some(1000 - i as i64 * 10);
        recs.push(r);
    }
    let mut empty = rec(9, "Plain message 5");
    empty.template = Some(String::new());
    recs.push(empty);
    recs.push(rec(10, "Plain message 7"));
    let p = mine(&recs, &MinerConfig::default());
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].template, "Handled request {RequestId} for user {User}");
    assert_eq!(p[0].source, TemplateSource::Serilog);
    assert_eq!(p[0].count, 4);
    assert_eq!(p[0].levels, [0, 0, 3, 0, 1, 0]);
    assert_eq!(p[0].first_ts, Some(970));
    assert_eq!(p[0].last_ts, Some(1000));
    assert_eq!(p[1].template, "Plain message <*>");
    assert_eq!(p[1].count, 2);
    assert_eq!(p[1].first_ts, None);
    let json = serde_json::to_value(&p[0]).unwrap();
    assert_eq!(json["sampleIds"].as_array().unwrap().len(), 4);
    assert_eq!(json["source"], "Serilog");
}

fn masked(s: &str) -> String {
    tokenize(s, 128).join(" ")
}

#[test]
fn masking_rules() {
    let cases = [
        ("took 23 ms", "took <*> ms"),
        ("took 23ms", "took <*>"),
        ("size 5MB, ok", "size <*>, ok"),
        ("delta -4.5 and +3", "delta <*> and <*>"),
        ("addr 0xDEADBEEF here", "addr <*> here"),
        ("hash 9f86d081884c7d65 done", "hash <*> done"),
        ("id 123e4567-e89b-12d3-a456-426614174000.", "id <*>."),
        ("id {123e4567-e89b-12d3-a456-426614174000}", "id {<*>}"),
        ("id a23e4567-e89b-12d3-a456-426614174000 x", "id <*> x"),
        ("hash deadbeef12 done", "hash <*> done"),
        ("peer 192.168.1.20:8080 up", "peer <*> up"),
        ("net 10.0.0.0/8", "net <*>"),
        ("v6 fe80::1ff:fe23:4567:890a x", "v6 <*> x"),
        ("at 2024-05-01T10:20:30.123Z end", "at <*> end"),
        ("at 2024-05-01 10:20:30,123 end", "at <*> <*> end"),
        ("mail bob.smith+x@example.co.uk sent", "mail <*> sent"),
        ("open /var/log/app.log now", "open <*> now"),
        ("open ./rel/x now", "open <*> now"),
        ("open ../x now", "open <*> now"),
        ("open ~/x now", "open <*> now"),
        ("open C:\\Users\\bob\\x.txt now", "open <*> now"),
        ("open D:/data now", "open <*> now"),
        ("open \\\\srv\\share now", "open <*> now"),
        ("get https://example.com/a?b=1 now", "get <*> now"),
        ("and/or a/b/c", "and/or <*>"),
        ("user=\"john doe\" ok", "user=<*> ok"),
        ("said \"hello there\", bye", "said <*>, bye"),
        ("said 'hello there' bye", "said <*> bye"),
        ("don't stop, isn't it", "don't stop, isn't it"),
        ("call(\"x y\") end", "call(<*>) end"),
        ("name='a b'", "name=<*>"),
        ("retry=3 count=abc key=", "retry=<*> count=abc key="),
        ("(42) [7] {x}", "(<*>) [<*>] {x}"),
        ("plain words only", "plain words only"),
        ("a - b", "a - b"),
        ("\"unterminated quote here", "\"unterminated quote here"),
        ("\"e\\\"scaped\" x", "<*> x"),
        ("  spaced   out  ", "spaced out"),
        ("''", "<*>"),
        ("naïve café 5", "naïve café <*>"),
        ("a@b", "a@b"),
        ("a@b.", "a@b."),
        ("a@b@c.d", "a@b@c.d"),
        ("@x.y", "@x.y"),
        ("1:2", "<*>"),
        ("abcdef12", "<*>"),
        ("0x", "<*>"),
        ("::", "::"),
        ("", ""),
    ];
    for (input, want) in cases {
        assert_eq!(masked(input), want, "input: {input:?}");
    }
    assert_eq!(tokenize("a b c d", 2).len(), 2);
    let long_quote = format!("x \"{}\" y", "z ".repeat(2000));
    assert!(masked(&long_quote).contains("\"z"));
}

// ---------------------------------------------------------------------------
// compare()
// ---------------------------------------------------------------------------

fn pats(items: &[(&str, usize)]) -> Vec<imt_core::patterns::Pattern> {
    let mut recs = Vec::new();
    for (t, n) in items {
        for _ in 0..*n {
            let mut r = rec(recs.len() as u32, "x");
            r.template = Some(t.to_string());
            recs.push(r);
        }
    }
    mine(&recs, &MinerConfig::default())
}

#[test]
fn compare_classifies_changes() {
    let baseline = pats(&[
        ("steady", 100),
        ("spiky", 10),
        ("dropping", 90),
        ("vanished", 20),
        ("tiny", 1),
        ("tiny-drop", 3),
    ]);
    let window = pats(&[
        ("steady", 100),
        ("spiky", 100),
        ("dropping", 4),
        ("newone", 7),
        ("tiny", 4),
        ("tiny-drop", 0),
    ]);
    let d = compare(&baseline, &window);
    let get = |t: &str| d.iter().find(|x| x.template == t).unwrap();
    assert_eq!(get("newone").change, Change::New);
    assert_eq!(get("newone").ratio, None);
    assert_eq!(get("vanished").change, Change::Gone);
    assert_eq!(get("vanished").window_count, 0);
    assert_eq!(get("spiky").change, Change::Spike);
    assert_eq!(get("spiky").baseline_count, 10);
    assert!(get("spiky").ratio.unwrap() > 3.0);
    assert_eq!(get("dropping").change, Change::Drop);
    assert!(get("dropping").ratio.unwrap() < 0.34);
    assert_eq!(get("steady").change, Change::Same);
    // Below the minimum counts nothing is flagged, even at 4x.
    assert_eq!(get("tiny").change, Change::Same);
    assert_eq!(get("tiny-drop").change, Change::Gone);
    // Ordering: New, Spike, Gone, Drop, Same.
    let order: Vec<Change> = d.iter().map(|x| x.change).collect();
    assert!(order.windows(2).all(|w| rank(w[0]) <= rank(w[1])), "{order:?}");
    assert_eq!(serde_json::to_value(get("spiky")).unwrap()["baselineCount"], 10);
    assert_eq!(serde_json::to_value(get("spiky")).unwrap()["change"], "Spike");
}

fn rank(c: Change) -> u8 {
    match c {
        Change::New => 0,
        Change::Spike => 1,
        Change::Gone => 2,
        Change::Drop => 3,
        Change::Same => 4,
    }
}

#[test]
fn compare_normalises_by_totals_and_handles_empty() {
    // Ten times the volume, same mix: nothing changes.
    let d = compare(&pats(&[("a", 10), ("b", 10)]), &pats(&[("a", 100), ("b", 100)]));
    assert!(d.iter().all(|x| x.change == Change::Same));
    assert!((d[0].ratio.unwrap() - 1.0).abs() < 1e-9);
    // Empty baseline: everything is new. Empty window: everything is gone.
    let w = pats(&[("a", 3)]);
    assert_eq!(compare(&[], &w)[0].change, Change::New);
    assert_eq!(compare(&w, &[])[0].change, Change::Gone);
    assert!(compare(&[], &[]).is_empty());
    // Same template present in several patterns of one side is summed.
    let mut two = pats(&[("a", 6)]);
    two.extend(pats(&[("a", 4)]));
    assert_eq!(compare(&two, &two)[0].baseline_count, 10);
}

// ---------------------------------------------------------------------------
// Robustness
// ---------------------------------------------------------------------------

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn garbage(rng: &mut Lcg) -> String {
    const PIECES: &[&str] = &[
        "at ",
        " in ",
        ":line ",
        "--->",
        "Caused by:",
        "Suppressed:",
        "... 3 more",
        "File \"",
        "\", line ",
        ", in ",
        "Traceback (most recent call last):",
        "--- End of inner exception stack trace ---",
        "(",
        ")",
        "[",
        "]",
        ":",
        "::",
        "<>c__",
        "b__0",
        "MoveNext",
        "`2",
        "$$Lambda$",
        "/",
        "\\",
        "\"",
        "'",
        "=",
        "@",
        ".",
        ",",
        "\n",
        "\r\n",
        "\t",
        "  ",
        "0x",
        "-",
        "é",
        "日本",
        "\u{0}",
        "Exception",
        "10.0.0.1",
        "123",
        "abc",
    ];
    let n = rng.next() % 40;
    (0..n).map(|_| PIECES[(rng.next() as usize) % PIECES.len()]).collect()
}

#[test]
fn never_panics_on_garbage() {
    let mut rng = Lcg(42);
    let mut miner = PatternMiner::new(MinerConfig::default());
    let mut records = Vec::new();
    for i in 0..4000u32 {
        let g = garbage(&mut rng);
        let mut r = rec(i, &g);
        r.exception = Some(garbage(&mut rng));
        r.template = if i % 7 == 0 { Some(garbage(&mut rng)) } else { None };
        let _ = parse_exception(&g);
        if let Some(e) = parse_exception(r.exception.as_deref().unwrap()) {
            let _ = fingerprint(&e, 5);
        }
        miner.add(&r);
        records.push(r);
    }
    assert!(!miner.clusters().is_empty());
    let keys: Vec<String> = (0..records.len()).map(|i| format!("k{}", i % 3)).collect();
    let groups = group(records.iter().zip(keys.iter().map(String::as_str)));
    assert_eq!(groups.iter().map(|g| g.count).sum::<usize>(), records.len());

    // Arbitrary bytes, including multi-byte characters next to quotes and separators.
    for _ in 0..3000 {
        let bytes: Vec<u8> = (0..(rng.next() % 80)).map(|_| (rng.next() % 256) as u8).collect();
        let s = String::from_utf8_lossy(&bytes);
        let _ = tokenize(&s, 128);
        let _ = parse_exception(&s);
    }
    // Pathological repeats.
    let _ = parse_exception(&"\"".repeat(100_000));
    let _ = tokenize(&"'a ".repeat(50_000), 128);
    let _ = parse_exception(&"at ".repeat(50_000));
}

#[test]
#[ignore = "benchmark: run with cargo test --release -- --ignored"]
fn mines_one_million_lines_quickly() {
    let start = std::time::Instant::now();
    let mut miner = PatternMiner::new(MinerConfig::default());
    let mut r = rec(0, "");
    for i in 0..1_000_000u32 {
        r.id = i;
        r.message = match i % 4 {
            0 => format!("Connected to 10.0.{}.{} in {} ms", i % 255, i % 200, i % 997),
            1 => format!("User user{} logged in from {}", i % 5000, i % 77),
            2 => format!("Request {} failed with status {}", i, 500 + i % 5),
            _ => "Cache miss".to_string(),
        };
        miner.add(&r);
    }
    let elapsed = start.elapsed();
    let patterns = miner.clusters();
    eprintln!("1M lines mined in {elapsed:?} into {} patterns", patterns.len());
    assert!(patterns.len() < 20);
    assert!(elapsed.as_secs() < 10, "took {elapsed:?}");
}

#[test]
fn the_miner_is_capped_and_counts_the_overflow() {
    // Regression (engine review finding 13): the Drain miner kept every distinct pattern.
    let mut miner = PatternMiner::with_limit(MinerConfig::default(), 3);
    let words = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"];
    for (i, w) in words.iter().enumerate() {
        let r = rec(i as u32, &format!("{w} {w} {w} {w}"));
        miner.add(&r);
    }
    let mut t = rec(9, "templated");
    t.template = Some("serilog {X}".into());
    miner.add(&t);
    let patterns = miner.clusters();
    assert_eq!(patterns.len(), 4, "{patterns:?}");
    assert_eq!(miner.overflowed(), 4);
    let overflow = patterns
        .iter()
        .find(|p| p.template == imt_core::patterns::OVERFLOW_TEMPLATE)
        .unwrap();
    assert_eq!(overflow.count, 4);
    // Lines matching a kept pattern still join it.
    miner.add(&rec(10, "alpha alpha alpha alpha"));
    assert_eq!(miner.overflowed(), 4);
}

// ---------------------------------------------------------------------------
// Trend sparklines and placeholder values (ADR 0046)
// ---------------------------------------------------------------------------

fn timed(id: u32, ts: i64, message: &str) -> Record {
    let mut r = rec(id, message);
    r.ts = Some(ts);
    r
}

#[test]
fn every_pattern_gets_a_trend_on_one_shared_scale() {
    let mut recs = Vec::new();
    // "steady" runs through the hour; "burst" happens only in the last ten minutes.
    for i in 0..60i64 {
        recs.push(timed(recs.len() as u32, i * 60_000, "heartbeat ok"));
    }
    for i in 0..30i64 {
        recs.push(timed(recs.len() as u32, 3_000_000 + i * 20_000, "cache miss storm"));
    }
    let mined = mine(&recs, &MinerConfig::default());
    let steady = mined.iter().find(|p| p.template == "heartbeat ok").unwrap();
    let burst = mined.iter().find(|p| p.template == "cache miss storm").unwrap();
    assert_eq!(steady.trend.len(), imt_core::patterns::TREND_BUCKETS);
    assert_eq!(
        (steady.trend_from, steady.trend_bucket_ms),
        (burst.trend_from, burst.trend_bucket_ms)
    );
    assert_eq!(steady.trend.iter().sum::<u32>(), 60);
    assert_eq!(burst.trend.iter().sum::<u32>(), 30);
    assert!(
        steady.trend.iter().filter(|&&n| n > 0).count() >= 20,
        "steady covers the range: {:?}",
        steady.trend
    );
    assert!(
        burst.trend.iter().filter(|&&n| n > 0).count() <= 6,
        "the burst is local: {:?}",
        burst.trend
    );
    assert!(burst.trend[..16].iter().all(|&n| n == 0));
}

#[test]
fn patterns_without_timestamps_have_no_trend() {
    let recs: Vec<Record> = (0..5).map(|i| rec(i, "no clock here")).collect();
    let mined = mine(&recs, &MinerConfig::default());
    assert!(mined[0].trend.is_empty());
    assert_eq!((mined[0].trend_from, mined[0].trend_bucket_ms), (None, 0));
}

#[test]
fn placeholders_list_the_values_behind_each_wildcard() {
    let mut recs = Vec::new();
    for i in 0..30u32 {
        let user = ["alice", "bob", "carol"][(i % 3) as usize];
        let msg = format!("Handled request {} for user {user} from 10.0.0.{}", 1000 + i, i % 2);
        recs.push(rec(i, &msg));
    }
    let mined = mine(&recs, &MinerConfig::default());
    assert_eq!(mined.len(), 1, "{:?}", mined.iter().map(|p| &p.template).collect::<Vec<_>>());
    let p = &mined[0];
    assert!(p.template.contains("<*>"), "{}", p.template);
    // The user name started as a literal and became a placeholder: the events before that count too.
    let user = p
        .placeholders
        .iter()
        .find(|h| h.values.iter().any(|v| v.value == "alice"))
        .expect("a placeholder for the user");
    assert!(user.exact && user.total == 30);
    let counts: Vec<(&str, u64)> = user.values.iter().map(|v| (v.value.as_str(), v.count)).collect();
    assert_eq!(counts, [("alice", 10), ("bob", 10), ("carol", 10)]);
    // The request numbers are masked from the start; their placeholder holds the raw values (30 distinct: bounded).
    let order = &p.placeholders[0];
    assert_eq!((order.index, order.total, order.exact), (0, 30, false));
    assert_eq!(user.index, 1, "indexes follow the template text");
    let ip = p
        .placeholders
        .iter()
        .find(|h| h.values.iter().any(|v| v.value == "10.0.0.1"))
        .unwrap();
    assert_eq!(ip.values.iter().map(|v| v.count).sum::<u64>(), 30);
}

#[test]
fn a_flood_of_distinct_values_stays_bounded_and_keeps_the_frequent_ones() {
    let mut recs = Vec::new();
    for i in 0..3000u32 {
        let id = if i % 3 == 0 { 7 } else { 100_000 + i };
        recs.push(rec(i, &format!("session {id} refreshed")));
    }
    let mined = mine(&recs, &MinerConfig::default());
    let p = &mined[0];
    let h = &p.placeholders[0];
    assert!(!h.exact);
    assert!(h.values.len() <= imt_core::patterns::TOP_VALUES);
    assert_eq!(h.values[0].value, "7");
    assert!(h.values[0].count >= 1000);
    assert_eq!(h.total, 3000);
}

#[test]
fn serilog_templates_and_long_values_are_handled() {
    let mut r = rec(0, "Order 5 placed");
    r.template = Some("Order {OrderId} placed".into());
    let mined = mine(&[r], &MinerConfig::default());
    assert!(
        mined[0].placeholders.is_empty(),
        "properties of a structured template are fields, not placeholders"
    );
    let long = "x".repeat(500);
    let mined = mine(
        &[
            rec(0, &format!("token=\"{long}\" accepted")),
            rec(1, &format!("token=\"{long}2\" accepted")),
        ],
        &MinerConfig::default(),
    );
    let v = &mined[0].placeholders[0].values[0].value;
    assert_eq!(v.chars().count(), imt_core::patterns::MAX_VALUE_CHARS);
}
