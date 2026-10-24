//! The run paths build.rs bakes into the worker (build/rpath.rs).

#[path = "../../build/rpath.rs"]
mod build_rpath;

#[test]
fn macos_finds_the_library_beside_the_worker_and_in_the_app_resources() {
    assert_eq!(
        build_rpath::rpaths("macos"),
        ["@executable_path", "@executable_path/../Resources/lib"]
    );
    assert_eq!(
        build_rpath::link_args("macos"),
        ["-Wl,-rpath,@executable_path", "-Wl,-rpath,@executable_path/../Resources/lib"]
    );
}

#[test]
fn linux_finds_the_library_beside_the_worker_and_in_the_package_resources() {
    assert_eq!(build_rpath::rpaths("linux"), ["$ORIGIN", "$ORIGIN/../lib/Debrief/lib"]);
    assert_eq!(
        build_rpath::link_args("linux"),
        ["-Wl,-rpath,$ORIGIN", "-Wl,-rpath,$ORIGIN/../lib/Debrief/lib"]
    );
}

#[test]
fn other_targets_get_no_run_paths() {
    for os in ["windows", "freebsd", "android", ""] {
        assert!(build_rpath::rpaths(os).is_empty(), "{os}");
        assert!(build_rpath::link_args(os).is_empty(), "{os}");
    }
}

#[test]
fn the_linux_path_names_the_apps_product_name() {
    let conf = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/src-tauri/tauri.conf.json")).unwrap();
    let conf: serde_json::Value = serde_json::from_str(&conf).unwrap();
    assert_eq!(conf["productName"].as_str(), Some(build_rpath::PRODUCT_NAME));
}

#[test]
fn the_resource_folder_is_lib() {
    let conf = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/src-tauri/tauri.sidecars.conf.json")).unwrap();
    let conf: serde_json::Value = serde_json::from_str(&conf).unwrap();
    assert_eq!(conf["bundle"]["resources"]["binaries/lib/"].as_str(), Some("lib/"));
}
