fn main() {
    // btmac is macOS-only (the crate body is cfg'd out elsewhere). Bail before
    // invoking clang so the crate stays harmless in a Linux workspace build.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    let shim_dir = "../../macos";
    println!("cargo:rerun-if-changed={shim_dir}/btshim.m");
    println!("cargo:rerun-if-changed={shim_dir}/btshim.h");

    cc::Build::new()
        .file(format!("{shim_dir}/btshim.m"))
        .include(shim_dir)
        .flag("-fobjc-arc")
        .compile("btshim");

    println!("cargo:rustc-link-lib=framework=IOBluetooth");
    println!("cargo:rustc-link-lib=framework=Foundation");
}
