#![no_main]
//! The `.imtplugin` reader (ADR 0027) on untrusted bytes: a package a user drops on IMT. Never panics; bounded by
//! the reader's limits.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = imt_plugin_package::Package::read(std::io::Cursor::new(data));
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = imt_plugin_package::PackageManifest::parse(text);
        let _ = imt_plugin_package::PublicKey::parse(text);
    }
});
