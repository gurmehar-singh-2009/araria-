//! Rebuilds the crate when the embedded `games/` catalog changes.

fn main() {
    println!("cargo:rerun-if-changed=../../games");
}
