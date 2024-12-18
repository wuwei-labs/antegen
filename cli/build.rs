use std::path::PathBuf;
use cargo_toml::Manifest;

fn main() {
    let plugin_cargo_path = PathBuf::from("../plugin/Cargo.toml");
    let manifest = Manifest::from_path(&plugin_cargo_path)
        .expect("Failed to read plugin Cargo.toml");

    let geyser_interface_version = manifest
        .dependencies
        .get("agave-geyser-plugin-interface")
        .expect("Unable to find agave-geyser-plugin-interface dependency")
        .req()
        .to_string();

    println!(
        "cargo:rustc-env=GEYSER_INTERFACE_VERSION={}",
        geyser_interface_version
    );
}
