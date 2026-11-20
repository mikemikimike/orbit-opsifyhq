// The documentation under `docs/` is embedded into the binary (see `src/docs.rs`).
// Re-run the build when it changes so new or removed pages are picked up.
fn main() {
    println!("cargo:rerun-if-changed=../../docs");
}
