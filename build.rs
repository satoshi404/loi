//! Generate the embedded library asset table used by `loi-lsp`.
//!
//! The language server is a separate executable that stages the standard
//! library into a cache directory at first run. Listing the assets here keeps
//! nested packages discoverable without a hard-coded allow-list.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let lib = manifest.join("lib");
    println!("cargo:rerun-if-changed={}", lib.display());

    let mut files = Vec::new();
    collect(&lib, &lib, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let modules: Vec<_> = files
        .iter()
        .filter(|(name, _)| name.ends_with(".loi"))
        .collect();
    let shims: Vec<_> = files
        .iter()
        .filter(|(name, _)| name.ends_with("_shim.c"))
        .collect();

    let mut out = String::from("const LIB_FILES: &[(&str, &str)] = &[\n");
    for (name, path) in modules {
        out.push_str(&format!(
            "    ({:?}, include_str!({:?})),\n",
            name,
            path.to_string_lossy()
        ));
    }
    out.push_str("];\n\nconst SHIM_FILES: &[(&str, &str)] = &[\n");
    for (name, path) in shims {
        out.push_str(&format!(
            "    ({:?}, include_str!({:?})),\n",
            name,
            path.to_string_lossy()
        ));
    }
    out.push_str("];\n");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    fs::write(out_dir.join("lib_assets.rs"), out).expect("generated assets should be writable");
}

fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, files);
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".loi") || name.ends_with("_shim.c") {
            let relative = path
                .strip_prefix(root)
                .expect("asset should be under lib")
                .to_string_lossy()
                .replace('\\', "/");
            files.push((relative, path));
        }
    }
}
