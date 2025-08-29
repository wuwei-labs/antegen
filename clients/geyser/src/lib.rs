use agave_geyser_plugin_interface::geyser_plugin_interface::GeyserPlugin;

mod builder;
mod events;
mod metrics;
mod plugin;
mod utils;

pub mod config {
    pub use crate::utils::PluginConfig;
}

pub use plugin::AntegenPlugin;

#[no_mangle]
#[allow(improper_ctypes_definitions)]
/// # Safety
///
/// The Solana validator and this plugin must be compiled with the same Rust compiler version and Solana core version.
/// Loading this plugin with mismatching versions is undefined behavior and will likely cause memory corruption.
pub unsafe extern "C" fn _create_plugin() -> *mut dyn GeyserPlugin {
    let plugin: Box<dyn GeyserPlugin> = Box::new(AntegenPlugin::default());
    Box::into_raw(plugin)
}
