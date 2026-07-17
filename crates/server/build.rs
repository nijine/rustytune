use std::fs;
use std::path::Path;

// rust-embed's derive fails compilation if the embed folder doesn't exist,
// which would break `cargo build` on a fresh clone before the first web
// build. Ensure it exists (empty is fine — the server just serves 404s
// until `npm run build` populates it).
fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
    fs::create_dir_all(&dist).expect("failed to create web/dist");
}
