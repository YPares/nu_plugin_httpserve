use nu_plugin::{Plugin, PluginCommand};

mod plugin;
mod serve;

pub use plugin::HTTPPlugin;

impl Plugin for HTTPPlugin {
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }

    fn commands(&self) -> Vec<Box<dyn PluginCommand<Plugin = Self>>> {
        vec![Box::new(crate::plugin::HTTPServe)]
    }
}
