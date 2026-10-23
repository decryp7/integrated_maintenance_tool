use super::*;

fn names(texts: &[&str], kind: EntityKind) -> Vec<String> {
    extract(texts)
        .entities
        .into_iter()
        .filter(|e| e.kind == kind)
        .map(|e| e.name)
        .collect()
}

#[test]
fn executables_libraries_and_files() {
    let t = "Started CompanyA.Graphic.Builder.exe and loaded libfoo.so.1.2 plus Widget.dll, wrote report.json (see notes.txt).";
    assert_eq!(names(&[t], EntityKind::Executable), ["CompanyA.Graphic.Builder.exe"]);
    assert_eq!(names(&[t], EntityKind::Library), ["libfoo.so.1.2", "Widget.dll"]);
    assert_eq!(names(&[t], EntityKind::File), ["report.json", "notes.txt"]);
    assert!(names(&["x libgl.dylib y driver.sys"], EntityKind::Library).len() == 2);
}

#[test]
fn paths_windows_and_posix() {
    let t =
        r#"open C:\Tools\Acme\run.exe then /var/log/acme/app.log and "C:\Program Files\Acme\x.dll" \\fileserver\share\a.txt ./rel/b.cfg"#;
    let paths = names(&[t], EntityKind::Path);
    assert!(paths.contains(&r"C:\Tools\Acme\run.exe".to_string()), "{paths:?}");
    assert!(paths.contains(&"/var/log/acme/app.log".to_string()));
    assert!(paths.contains(&r"C:\Program Files\Acme\x.dll".to_string()));
    assert!(paths.contains(&"./rel/b.cfg".to_string()));
    assert!(names(&[t], EntityKind::Host).contains(&"fileserver".to_string()));
    assert!(names(&[t], EntityKind::Executable).contains(&"run.exe".to_string()));
    assert!(names(&[t], EntityKind::Library).contains(&"x.dll".to_string()));
    assert!(
        names(&["GET /health ok"], EntityKind::Path).is_empty(),
        "a single segment is not a path"
    );
}

#[test]
fn hosts_ips_and_urls() {
    let t = "connect api.example.com:8443 via 10.0.0.7:5000 and [::1] or fe80::1 then https://user:pw@Cdn.Example.org:8080/a/b?token=secret#frag failed";
    assert_eq!(names(&[t], EntityKind::Ip), ["10.0.0.7", "::1", "fe80::1",]);
    let hosts = names(&[t], EntityKind::Host);
    assert!(
        hosts.contains(&"api.example.com".to_string()) && hosts.contains(&"cdn.example.org".to_string()),
        "{hosts:?}"
    );
    assert_eq!(
        names(&[t], EntityKind::Url),
        ["https://cdn.example.org:8080/a/b"],
        "no user info, query or fragment"
    );
    assert!(names(&["at 12:30:45 mac aa:bb:cc:dd:ee:ff"], EntityKind::Ip).is_empty());
    assert!(names(&["System.Timeout app.json v1.2.3"], EntityKind::Host).is_empty());
    assert_eq!(names(&["http://[2001:db8::1]:80/x"], EntityKind::Ip), ["2001:db8::1"]);
    assert!(names(&["http:// ://x"], EntityKind::Url).is_empty());
}

#[test]
fn exceptions_and_codes() {
    let t = "System.TimeoutException: boom; ValueError raised, HRESULT 0x80070005 and 0xC0000005, errno ENOENT, ERROR_ACCESS_DENIED, status 503, returned 404 but 200 widgets, version 0x1234";
    assert_eq!(names(&[t], EntityKind::Exception), ["System.TimeoutException", "ValueError"]);
    assert_eq!(
        names(&[t], EntityKind::ErrorCode),
        ["0x80070005", "0xC0000005", "ENOENT", "ERROR_ACCESS_DENIED", "HTTP 503", "HTTP 404"]
    );
    assert!(
        names(&["Error Exception"], EntityKind::Exception).is_empty(),
        "generic words are not types"
    );
}

#[test]
fn guids_are_counted() {
    let t = "ids 3f2504e0-4f89-11d3-9a0c-0305e82c3301 and 3F2504E0-4F89-11D3-9A0C-0305E82C3301 and 00000000-0000-0000-0000-000000000001";
    let e = extract(&[t]);
    assert_eq!(e.guids, 2, "case-insensitive duplicates are one");
}

#[test]
fn bounded_and_never_panics() {
    let many: String = (0..500).map(|i| format!("host{i}.example.com ")).collect();
    assert_eq!(extract(&[&many]).entities.len(), MAX_PER_EVENT);
    let long = format!("{}.exe", "a".repeat(400));
    assert!(extract(&[&long]).entities.is_empty(), "over-long tokens are skipped");
    let late = format!("{} late.exe", "x ".repeat(MAX_SCAN_CHARS));
    assert!(extract(&[&late]).entities.is_empty(), "only the first characters are read");
    for s in [
        "",
        "://",
        "\"",
        "\"unterminated C:\\x",
        "[::",
        "0x",
        "é.exe ü://ß",
        "\\\\",
        "~/",
        "a.",
        "...",
        "\u{0}\u{1}",
        "://:@",
    ] {
        let _ = extract(&[s]);
    }
}

#[test]
fn deterministic_and_deduplicated() {
    let t = "a.exe A.EXE a.exe";
    assert_eq!(extract(&[t]), extract(&[t]));
    assert_eq!(names(&[t], EntityKind::Executable), ["a.exe"]);
    assert_eq!(
        extract(&["x.dll", "y.dll x.dll"]).entities.len(),
        2,
        "across the message and the exception"
    );
}

#[test]
fn kinds_round_trip() {
    for k in [
        EntityKind::Executable,
        EntityKind::Library,
        EntityKind::File,
        EntityKind::Path,
        EntityKind::Host,
        EntityKind::Ip,
        EntityKind::Url,
        EntityKind::Exception,
        EntityKind::ErrorCode,
        EntityKind::Guid,
    ] {
        assert_eq!(EntityKind::from_id(k.id()), Some(k));
    }
    assert_eq!(EntityKind::from_id("nope"), None);
}

#[test]
fn three_digit_numbers_are_codes_only_in_an_http_context_and_range() {
    assert!(names(&["wrote 999 rows"], EntityKind::ErrorCode).is_empty());
    assert!(names(&["waited 404 ms"], EntityKind::ErrorCode).is_empty());
    assert_eq!(names(&["status 404 returned"], EntityKind::ErrorCode), ["HTTP 404"]);
}

#[test]
fn names_that_are_not_hosts_are_left_out() {
    assert!(names(&["see foo_bar.example now"], EntityKind::Host).is_empty());
    assert!(names(&["see foo$bar.example now"], EntityKind::Host).is_empty());
}

#[test]
fn quoted_stretches_are_split_or_kept_whole() {
    // A quoted stretch without spaces is split as usual; one with spaces is a single token.
    assert_eq!(names(&[r#"loaded "Widget.dll""#], EntityKind::Library), ["Widget.dll"]);
    assert_eq!(names(&[r#"loaded "a.dll,b.dll""#], EntityKind::Library), ["a.dll", "b.dll"]);
    // A quote that never closes within the token limit is dropped without a panic.
    let long = format!("\"{}", "a".repeat(10_000));
    let _ = extract(&[long.as_str()]);
}

#[test]
fn urls_with_odd_authorities() {
    assert!(names(&["GET http:///x done"], EntityKind::Url).is_empty());
    let _ = extract(&[
        "GET http://host:abc/x done",
        "GET http://[::1]:8080/x done",
        "GET http://[::1/x done",
    ]);
    assert!(names(&["GET http://host:abc/x done"], EntityKind::Url).len() <= 1);
}
