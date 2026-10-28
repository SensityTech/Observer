// The panel is embedded from web/dist: rebuild whenever `cargo xtask web` rewrites it, including the
// first time it appears.
fn main() {
    println!("cargo::rerun-if-changed=../../web/dist");
}
