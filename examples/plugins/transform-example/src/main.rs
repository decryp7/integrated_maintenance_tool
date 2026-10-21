//! The plugin's program: IMT starts it and talks to it over its private connection.

fn main() -> std::io::Result<()> {
    imt_plugin_api::serve(std::sync::Arc::new(imt_example_transform::RedactPlugin))
}
