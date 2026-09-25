use std::path::Path;

/// Monotonic build number: incremented only when `build.rs` actually reruns,
/// which happens when the sources change (cargo scans `src`/`native`). A plain
/// no-change rebuild keeps the same number, so incremental builds stay fast.
fn next_build_number() -> u64 {
    let path = Path::new("build_number");
    let current: u64 = std::fs::read_to_string(path)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(1000);
    let next = current + 1;
    let _ = std::fs::write(path, next.to_string());
    next
}

fn main() {
    println!("cargo:rerun-if-changed=native/libtorrent_bridge.cpp");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=native");
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("native/libtorrent_bridge.cpp")
        .compile("rextto_libtorrent_bridge");
    println!("cargo:rustc-link-lib=torrent-rasterbar");
    println!("cargo:rustc-link-lib=ssl");
    println!("cargo:rustc-link-lib=crypto");
    // Look for a bundled libtorrent next to the executable (`lib/`), which is how
    // the standalone release archive ships it. When the directory is absent (as
    // in development) the loader simply falls back to the system library.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/lib");
    }
    println!("cargo:rustc-env=REXTTO_BUILD={}", next_build_number());
}
