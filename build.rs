fn main() {
    println!("cargo:rerun-if-changed=assets");
    let v = |k: &str| std::env::var(k).expect("cargo sets the package version");
    embed_resource::compile(
        "assets/app.rc",
        [
            format!("VER_MAJOR={}", v("CARGO_PKG_VERSION_MAJOR")),
            format!("VER_MINOR={}", v("CARGO_PKG_VERSION_MINOR")),
            format!("VER_PATCH={}", v("CARGO_PKG_VERSION_PATCH")),
            format!("VER_STR=\"{}\"", v("CARGO_PKG_VERSION")),
        ],
    )
    .manifest_required()
    .expect("compile Windows resources");
}
