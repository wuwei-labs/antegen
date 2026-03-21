fn main() {
    // Auto-enable dev feature in debug builds unless prod is explicitly set
    let profile = std::env::var("PROFILE").unwrap_or_default();
    if profile == "debug" && !cfg!(feature = "prod") {
        println!("cargo:rustc-cfg=feature=\"dev\"");
    }
}
