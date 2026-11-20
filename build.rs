fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let root = std::env::var("CARGO_MANIFEST_DIR").expect("Cargo package directory");
        println!("cargo:rustc-link-arg-bin=NestedUnpack=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=NestedUnpack=/MANIFESTINPUT:{root}/app.manifest");
    }
}
