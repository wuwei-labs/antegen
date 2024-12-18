use std::path::PathBuf;
extern crate cargo_toml;
use cargo_toml::Manifest;
use rustc_version::version;

fn main() {
    let version = version().unwrap();
    println!("cargo:rustc-env=RUSTC_VERSION={}", version);

    let plugin_cargo_path = PathBuf::from("./Cargo.toml");
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
