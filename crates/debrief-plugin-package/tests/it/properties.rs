//! The package reader never panics, whatever the bytes: arbitrary input, and valid packages with bytes flipped or
//! cut off. A changed package either fails to read or still verifies (a flipped byte in a compressed stream may
//! decode to the same content); it is never accepted with content other than what was signed.

use debrief_plugin_package::archive::{build, Build};
use debrief_plugin_package::{Package, PackageManifest, SecretKey};
use proptest::prelude::*;
use std::io::Cursor;

const MANIFEST: &str = r#"format = 1
[plugin]
id = "com.example.prop"
name = "Prop"
version = "1.0.0"
publisher = "Example"
protocol = "1.0"
binary = "prop"
targets = ["x86_64-pc-windows-msvc"]
"#;

fn package(signed: bool) -> (Vec<u8>, String) {
    let t = tempfile::tempdir().unwrap();
    let prog = t.path().join("prop.exe");
    std::fs::write(&prog, vec![7u8; 3000]).unwrap();
    let out = t.path().join("p.debriefplugin");
    let key = SecretKey::generate().unwrap();
    let digest = build(
        Build {
            manifest: PackageManifest::parse(MANIFEST).unwrap(),
            programs: vec![("x86_64-pc-windows-msvc".into(), &prog)],
            sign: signed.then_some(&key),
        },
        &out,
    )
    .unwrap();
    (std::fs::read(out).unwrap(), digest)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = Package::read(Cursor::new(bytes));
    }

    #[test]
    fn damaged_packages_never_panic_or_verify_other_content(signed in any::<bool>(), flips in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8), cut in any::<prop::sample::Index>()) {
        let (good, digest) = package(signed);
        let mut bad = good.clone();
        for (i, x) in &flips {
            let i = i.index(bad.len());
            bad[i] ^= x | 1;
        }
        if let Ok(p) = Package::read(Cursor::new(&bad)) {
            // Accepted only with exactly the content that was built.
            prop_assert_eq!(p.digest(), digest.clone());
        }
        let truncated = &good[..cut.index(good.len())];
        let _ = Package::read(Cursor::new(truncated));
    }
}
