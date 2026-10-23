//! The importer example's program (`examples/plugins/importer-example`), for the end-to-end tests.

fn main() -> std::io::Result<()> {
    debrief_plugin_api::serve(std::sync::Arc::new(debrief_example_importer::KvLinesPlugin))
}
