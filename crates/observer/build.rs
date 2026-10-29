// On Windows the executable carries its icon, its version information and its manifest, which
// asks for no elevation, per-monitor DPI awareness, long paths and UTF-8.
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
}

#[cfg(windows)]
fn windows_resources() {
    const ICON: &str = "assets/observer.ico";
    const MANIFEST: &str = "assets/observer.manifest";
    for path in [ICON, MANIFEST] {
        println!("cargo::rerun-if-changed={path}");
    }
    let version = format!("{}.0", env!("CARGO_PKG_VERSION"));
    let mut resources = winresource::WindowsResource::new();
    resources
        .set_icon_with_id(ICON, "1")
        .set_manifest_file(MANIFEST)
        .set("FileVersion", &version)
        .set("ProductVersion", &version)
        .set("ProductName", "Observer")
        .set("FileDescription", "Observer, a server manager for FiveM and RedM")
        .set("OriginalFilename", "observer.exe")
        .set("InternalName", "observer");
    if let Err(error) = resources.compile() {
        panic!("cannot embed the Windows resources (the Windows SDK's rc.exe): {error}");
    }
}
