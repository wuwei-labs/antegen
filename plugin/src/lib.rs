pub mod config {
    pub use antegen_plugin_utils::PluginConfig;
}

use agave_geyser_plugin_interface::geyser_plugin_interface::GeyserPlugin;

mod builders;
mod events;
mod executors;
mod observers;
mod plugin;
mod pool_position;
mod utils;

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
