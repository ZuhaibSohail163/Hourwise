// Re-embed migrations whenever a file in ./migrations changes.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
