use nu_plugin::{serve_plugin, JsonSerializer};

fn main() {
    let plugin = nu_plugin_httpserve::HTTPServePlugin::new();
    serve_plugin(&plugin, JsonSerializer)
}
