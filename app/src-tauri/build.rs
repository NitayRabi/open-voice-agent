use std::path::Path;

fn main() {
    // The frontend is TypeScript; `webserver.rs` embeds the compiled output with
    // `include_dir!`, so it has to exist before rustc runs. `cargo tauri dev/build`
    // handles this via beforeDevCommand/beforeBuildCommand — a bare `cargo build`
    // needs it done by hand.
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist");
    if !dist.join("index.html").is_file() {
        panic!(
            "frontend build output missing at {}\n\
             run `npm ci && npm run build` in the app/ directory first",
            dist.display()
        );
    }
    println!("cargo:rerun-if-changed=../dist");

    tauri_build::build()
}
