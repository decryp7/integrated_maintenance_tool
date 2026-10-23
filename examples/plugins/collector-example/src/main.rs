//! The plugin's program: Debrief starts it and talks to it over its private connection.

fn main() -> std::io::Result<()> {
    debrief_plugin_api::serve(std::sync::Arc::new(debrief_example_collector::HeartbeatPlugin))
}
