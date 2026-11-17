use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Compress a vendor file into OUT_DIR and expose its content hash as an env var.
fn pack(src: &Path, out_name: &str, hash_env: &str, level: i32) {
    println!("cargo:rerun-if-changed={}", src.display());
    let raw = std::fs::read(src).unwrap_or_else(|e| {
        panic!("{} is missing ({e}). See docs/desktop-app-spec.md for how to produce it.", src.display())
    });
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut hasher);
    let hash = format!("{:016x}-{level}", hasher.finish());
    println!("cargo:rustc-env={hash_env}={}", &hash[..16]);
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join(out_name);
    // Reruns when either vendor file changes; skip what is already packed.
    let stamp = out.with_extension("hash");
    if out.exists() && std::fs::read_to_string(&stamp).ok().as_deref() == Some(hash.as_str()) {
        return;
    }

    let mut enc = zstd::stream::Encoder::new(Vec::new(), level).unwrap();
    enc.multithread(std::thread::available_parallelism().map_or(4, |n| n.get() as u32)).unwrap();
    enc.long_distance_matching(true).unwrap();
    std::io::copy(&mut raw.as_slice(), &mut enc).unwrap();
    std::fs::write(&out, enc.finish().unwrap()).unwrap();
    std::fs::write(stamp, hash).unwrap();
}

fn main() {
    let vendor = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("vendor");
    // Release builds squeeze harder; debug builds stay quick.
    let level = if std::env::var("PROFILE").as_deref() == Ok("release") { 19 } else { 3 };
    pack(&vendor.join("libmpv").join("libmpv-2.dll"), "libmpv-2.dll.zst", "LIBMPV_HASH", level);
    // The helper zip is already deflated; zstd on top only adds a little.
    pack(&vendor.join("helper").join("tapo-helper.zip"), "tapo-helper.zip.zst", "HELPER_HASH", 3);
    tauri_build::build()
}
