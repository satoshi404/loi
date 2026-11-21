//! `loi-lsp` — the Loi language server, shipped as a sibling of `loi`.
//!
//! Like `clangd`, this is a separate executable that editors start over stdio.
//! The server itself is written in Loi; this launcher keeps a compiled copy in
//! a cache directory, rebuilding it when the embedded sources change.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SERVER_SOURCE: &str = include_str!("../../examples/lsp_server.loi");

const LIB_FILES: &[(&str, &str)] = &[
    ("crc32.loi", include_str!("../../lib/crc32.loi")),
    ("ctype.loi", include_str!("../../lib/ctype.loi")),
    ("hash.loi", include_str!("../../lib/hash.loi")),
    ("json.loi", include_str!("../../lib/json.loi")),
    ("jsonrpc.loi", include_str!("../../lib/jsonrpc.loi")),
    ("lsp.loi", include_str!("../../lib/lsp.loi")),
    ("map.loi", include_str!("../../lib/map.loi")),
    ("math.loi", include_str!("../../lib/math.loi")),
    ("net.loi", include_str!("../../lib/net.loi")),
    ("option.loi", include_str!("../../lib/option.loi")),
    ("os.loi", include_str!("../../lib/os.loi")),
    ("process.loi", include_str!("../../lib/process.loi")),
    ("random.loi", include_str!("../../lib/random.loi")),
    ("result.loi", include_str!("../../lib/result.loi")),
    ("slice.loi", include_str!("../../lib/slice.loi")),
    ("std.loi", include_str!("../../lib/std.loi")),
    ("stdio.loi", include_str!("../../lib/stdio.loi")),
    ("string.loi", include_str!("../../lib/string.loi")),
    ("vec.loi", include_str!("../../lib/vec.loi")),
];

const SHIM_FILES: &[(&str, &str)] = &[
    ("lsp_shim.c", include_str!("../../lib/lsp_shim.c")),
    ("os_shim.c", include_str!("../../lib/os_shim.c")),
    ("process_shim.c", include_str!("../../lib/process_shim.c")),
    ("stdio_shim.c", include_str!("../../lib/stdio_shim.c")),
];

fn main() {
    if let Err(error) = run() {
        eprintln!("loi-lsp: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let compiler = compiler()?;
    let cache = cache_dir()?;
    fs::create_dir_all(&cache).map_err(|error| format!("{}: {error}", cache.display()))?;

    let fingerprint = fingerprint();
    let server = cache.join(format!("loi-lsp-{fingerprint}"));
    if !server.is_file() {
        build_server(&compiler, &cache, &server, &fingerprint)?;
    }

    let mut command = Command::new(&server);
    command.env("LOI_COMPILER", &compiler);
    command.args(env::args_os().skip(1));
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        Err(format!("could not start {}: {error}", server.display()))
    }

    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|error| format!("could not start {}: {error}", server.display()))?;
        if status.success() {
            Ok(())
        } else {
            std::process::exit(status.code().unwrap_or(1));
        }
    }
}

/// Prefer `LOI_COMPILER`, then the compiler installed next to this launcher,
/// then `loi` from PATH. The server inherits the chosen path so its own
/// `loi check` calls use the same compiler.
fn compiler() -> Result<PathBuf, String> {
    if let Some(explicit) = env::var_os("LOI_COMPILER") {
        return Ok(PathBuf::from(explicit));
    }
    let current = env::current_exe().map_err(|error| format!("current_exe: {error}"))?;
    let sibling = current.with_file_name("loi");
    if sibling.is_file() {
        return sibling
            .canonicalize()
            .map_err(|error| format!("{}: {error}", sibling.display()));
    }
    Ok(PathBuf::from("loi"))
}

fn cache_dir() -> Result<PathBuf, String> {
    if let Some(explicit) = env::var_os("LOI_LSP_CACHE_DIR") {
        return Ok(PathBuf::from(explicit));
    }
    if let Some(home) = env::var_os("HOME") {
        return Ok(PathBuf::from(home).join(".cache").join("loi").join("lsp"));
    }
    Ok(PathBuf::from("/tmp/loi-lsp-cache"))
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100_0000_01b3);
    }
}

fn fingerprint() -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    hash_bytes(&mut hash, env!("CARGO_PKG_VERSION").as_bytes());
    hash_bytes(&mut hash, SERVER_SOURCE.as_bytes());
    for (name, source) in LIB_FILES {
        hash_bytes(&mut hash, name.as_bytes());
        hash_bytes(&mut hash, source.as_bytes());
    }
    for (name, source) in SHIM_FILES {
        hash_bytes(&mut hash, name.as_bytes());
        hash_bytes(&mut hash, source.as_bytes());
    }
    format!("{hash:016x}")
}

fn build_server(
    compiler: &Path,
    cache: &Path,
    server: &Path,
    fingerprint: &str,
) -> Result<(), String> {
    let staging = cache.join(format!("build-{fingerprint}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    let examples = staging.join("examples");
    let lib = staging.join("lib");
    fs::create_dir_all(&examples).map_err(|error| format!("{}: {error}", examples.display()))?;
    fs::create_dir_all(&lib).map_err(|error| format!("{}: {error}", lib.display()))?;
    fs::write(examples.join("loi-lsp.loi"), SERVER_SOURCE)
        .map_err(|error| format!("could not write the server source: {error}"))?;
    for (name, source) in LIB_FILES {
        fs::write(lib.join(name), source)
            .map_err(|error| format!("could not write lib/{name}: {error}"))?;
    }
    for (name, source) in SHIM_FILES {
        fs::write(lib.join(name), source)
            .map_err(|error| format!("could not write lib/{name}: {error}"))?;
    }

    let pending = cache.join(format!("loi-lsp-{fingerprint}.{}", std::process::id()));
    let mut command = Command::new(compiler);
    command
        .arg("build")
        .arg("examples/loi-lsp.loi")
        .arg("-o")
        .arg(&pending)
        .arg("--error-format=short")
        .arg("--color=never")
        .current_dir(&staging);
    for (name, _) in SHIM_FILES {
        command.arg("--c-source").arg(format!("lib/{name}"));
    }
    let output = command
        .output()
        .map_err(|error| format!("could not run {}: {error}", compiler.display()))?;
    if !output.status.success() {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!(
            "could not compile the language server:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    fs::rename(&pending, server)
        .map_err(|error| format!("could not install {}: {error}", server.display()))?;
    let _ = fs::remove_dir_all(&staging);
    Ok(())
}
