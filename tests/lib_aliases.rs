use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn repo_lib() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lib")
}

fn test_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "loi-lib-aliases-{}-{name}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

/// Aliases introduced after the logical-import work. Prefixed names are not
/// listed: they are the compatibility surface and are covered by existing
/// tests. A `view` alias is deliberately absent (it would shadow the built-in
/// call inside `std/vec.loi`).
fn planned_aliases() -> Vec<(&'static str, &'static [&'static str])> {
    vec![
        ("std/fmt.loi", &["label", "line", "parens"]),
        (
            "std/vec.loi",
            &[
                "new",
                "size",
                "capacity",
                "is_empty",
                "reserve",
                "clear",
                "get",
                "set",
                "push",
                "pop",
                "find",
                "from_slice",
                "as_slice",
                "reverse",
                "sort",
                "binary_search",
            ],
        ),
        (
            "std/str.loi",
            &[
                "concat",
                "contains",
                "count",
                "eq",
                "index_of",
                "is_ascii_space",
                "repeat",
                "starts_with",
                "trim",
            ],
        ),
        // `copy` is a common local name in `std/vec.loi`; keep the prefixed
        // `slice_copy` there instead of introducing a program-wide `copy`.
        ("std/slice.loi", &["first", "last", "equals_u8"]),
        ("std/iter.loi", &["sum_u64", "any_i64", "all_positive_i64"]),
        (
            "std/map.loi",
            &[
                "int_new",
                "int_len",
                "int_capacity",
                "int_contains",
                "int_get",
                "int_get_or",
                "int_lookup",
                "int_set",
                "int_remove",
                "int_clear",
                "str_new",
                "str_capacity",
                "str_get",
                "str_get_or",
                "str_lookup",
                "str_set",
                "str_remove",
                "str_clear",
            ],
        ),
        (
            "std/hash.loi",
            &["bytes_seeded", "bytes", "u64_seeded", "pair_u64"],
        ),
        // `bytes` stays owned by `std::hash` in the broad `std` root; the
        // prefixed `crc32_bytes` name is already short enough.
        ("std/crc32.loi", &["update", "extend"]),
        (
            "std/ctype.loi",
            &[
                "is_digit",
                "is_upper",
                "is_lower",
                "is_alpha",
                "is_alnum",
                "is_hex_digit",
                "is_space",
                "is_ascii",
                "to_lower",
                "to_upper",
                "hex_value",
                "parse_u64",
                "count_digits",
                "equals_ignore_case",
            ],
        ),
        // `state` is a local name in `std/crc32.loi`, so only the genuinely
        // distinct random helpers get unprefixed aliases.
        (
            "std/random.loi",
            &["next_u64", "next_u32", "below_u64", "pick", "shuffle"],
        ),
        (
            "std/debug.loi",
            &["assert", "expect_i64_eq", "expect_u64_lt"],
        ),
        ("core/cmp.loi", &["eq", "lt", "clamp_u64", "max_u64"]),
        ("core/mem.loi", &["zeroed_bytes", "fill_u8", "copy_u8"]),
        (
            "core/ops.loi",
            &[
                "is_power_of_two_u64",
                "next_power_of_two_u64",
                "saturating_sub_u64",
            ],
        ),
        (
            "core/option.loi",
            &["none", "some", "is_some", "is_none", "get_or", "unwrap"],
        ),
        (
            "core/result.loi",
            &["ok", "err", "is_ok", "is_err", "unwrap_or", "unwrap_err"],
        ),
        (
            "os/env.loi",
            &["pid", "has_env", "env_i32", "env", "errno", "exit"],
        ),
        (
            "os/file.loi",
            &["release", "open_read", "open_write", "read", "write"],
        ),
        (
            "os/path.loi",
            &["is_absolute", "file_name", "parent", "extension"],
        ),
        ("os/time.loi", &["unix_time", "sleep"]),
        (
            "os/process.loi",
            &[
                "release",
                "spawn_args",
                "spawn",
                "ok",
                "stdout_len",
                "stderr_len",
                "read_stdout",
                "read_stderr",
                "stdout",
                "stderr",
                "exit_code",
            ],
        ),
        ("os/stdio.loi", &["read_exact", "write_all", "flush"]),
        (
            "net/address.loi",
            &["ipv4", "ipv4_octet", "port_is_valid", "parse_ipv4"],
        ),
        ("net/socket.loi", &["tcp_connect", "close", "send", "recv"]),
        (
            "net/tcp.loi",
            &[
                "stream_release",
                "stream_connect",
                "stream_send",
                "stream_recv",
            ],
        ),
        (
            "net/udp.loi",
            &[
                "endpoint",
                "endpoint_port",
                "endpoint_is_broadcast",
                "endpoint_is_loopback",
            ],
        ),
        // `bytes` is a local name in `encoding/json.loi`; `hash::bytes` owns
        // that short name in programs that load both packages.
        ("runtime/arena.loi", &["filled_bytes"]),
        (
            "runtime/hooks.loi",
            &["none", "with_shutdown", "wants_setup", "wants_shutdown"],
        ),
        ("runtime/panic.loi", &["assert", "assert_i64_eq"]),
        (
            "encoding/json.loi",
            // `root` is a local name in `std/vec.loi`, so `json_root` stays
            // prefixed to avoid ambiguity in mixed programs.
            &[
                "parse",
                "kind",
                "number_text",
                "number_i64",
                "array_len",
                "array_get",
                "object_get",
                "stringify",
                "new",
                "new_null",
                "new_bool",
                "new_number",
                "new_i64",
                "new_u64",
                "new_string",
                "new_array",
                "new_object",
                "array_push",
                "object_set",
                "set_root",
            ],
        ),
        (
            "encoding/jsonrpc.loi",
            &[
                "read_frame",
                "parse_request",
                "send_result",
                "send_notification",
                "send_error",
            ],
        ),
        (
            "lsp/protocol.loi",
            &["publish_diagnostics", "send_completion"],
        ),
        (
            "lsp/shim.loi",
            &["write_document", "remove_document", "read_document"],
        ),
        (
            "math/float.loi",
            &[
                "clamp_f64",
                "lerp",
                "smoothstep",
                "deg_to_rad",
                "rad_to_deg",
                "hypot",
                "near",
                "dot_f64",
            ],
        ),
        ("math/generic.loi", &["min", "max", "clamp", "sum"]),
        ("math/integer.loi", &["sign_i32", "gcd_u64", "pow_u64"]),
    ]
}

fn is_declared(source: &str, alias: &str) -> bool {
    let needle = format!("open flow {alias}");
    source.lines().any(|line| {
        let line = line.trim_start();
        line == needle
            || line.starts_with(&format!("{needle}<"))
            || line.starts_with(&format!("{needle} "))
    })
}

#[test]
fn planned_short_aliases_are_declared() {
    let mut missing = Vec::new();
    for (file, aliases) in planned_aliases() {
        let path = repo_lib().join(file);
        let source = fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        for alias in aliases {
            if !is_declared(&source, alias) {
                missing.push(format!("{}::{}", file.trim_end_matches(".loi"), alias));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "missing aliases: {}",
        missing.join(", ")
    );
}

#[test]
fn broad_package_roots_still_load() {
    let dir = test_dir("roots");
    let roots = [
        "std.loi",
        "core.loi",
        "os.loi",
        "net.loi",
        "runtime.loi",
        "encoding.loi",
        "math.loi",
        "lsp.loi",
    ];
    for root in roots {
        let source = format!(
            "import \"{}\"\nopen flow main\n    => i32\n{{\n    => 0\n}}\n",
            repo_lib().join(root).display()
        );
        fs::write(dir.join("main.loi"), source).expect("source should be writable");
        let output = Command::new(loi())
            .arg("check")
            .arg("main.loi")
            .current_dir(&dir)
            .output()
            .expect("loi should run");
        assert!(
            output.status.success(),
            "{root} failed to load: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

const NARROW_SOURCE: &str = "import std::{vec, str, slice, iter, map, hash, ctype, crc32, random, fmt}\n\
                             import core::{option, result, cmp, mem, ops}\n\
                             import math::{generic, integer}\n\
                             import runtime::{arena, hooks}\n\
                             import encoding::json\n\
                             import os::{path, env, time}\n\
                             import net::address\n\
                             open flow main\n\
                             {\n\
                                 protect {\n\
                                     mut text: [u8] <- str::concat(\"a\", \"b\")\n\
                                     n <- str::count(text, 98 as u8)\n\
                                     first <- slice::first<u64>([1 as u64, 2 as u64])\n\
                                     total <- iter::sum_u64([1 as u64, 2 as u64])\n\
                                     mut m: IntMap<u64>\n\
                                     map::int_new<u64>(-> m)\n\
                                     h <- hash::bytes(text)\n\
                                     crc <- crc32::update(0, text)\n\
                                     mut rng <- random::rng_new(1)\n\
                                     x <- random::next_u64(<-> rng)\n\
                                     d <- ctype::is_digit(48 as u8)\n\
                                     mut o: Option<u64>\n\
                                     option::some<u64>(7, -> o)\n\
                                     is_some_value <- option::is_some<u64>(o)\n\
                                     mut r: Result<u64, [u8]>\n\
                                     result::ok<u64, [u8]>(7, -> r)\n\
                                     is_ok_value <- result::is_ok<u64, [u8]>(r)\n\
                                     mx <- cmp::max_u64(1, 2)\n\
                                     mut buf: [u8]\n\
                                     mem::zeroed_bytes(-> buf, 4)\n\
                                     pow2 <- ops::is_power_of_two_u64(8)\n\
                                     small <- generic::min<i32>(1, 2)\n\
                                     sg <- integer::sign_i32(-3)\n\
                                     mut ab: [u8]\n\
                                     arena::filled_bytes(-> ab, 4, 65)\n\
                                     mut hk: RuntimeHooks\n\
                                     hooks::none(-> hk)\n\
                                     mut doc: JsonDoc\n\
                                     json::new(-> doc)\n\
                                     abs <- path::is_absolute(\"/tmp\")\n\
                                     pid <- env::pid()\n\
                                     now <- time::unix_time()\n\
                                     ip <- address::ipv4(127, 0, 0, 1)\n\
                                     _ <- fmt::line(text)\n\
                                 }\n\
                             }\n";

#[test]
fn representative_aliases_resolve_in_narrow_imports() {
    let dir = test_dir("narrow");
    fs::write(dir.join("main.loi"), NARROW_SOURCE).expect("source should be writable");
    let output = Command::new(loi())
        .args([
            "check",
            "main.loi",
            "--lib-dir",
            repo_lib().to_str().unwrap(),
        ])
        .current_dir(&dir)
        .output()
        .expect("loi should run");
    assert!(
        output.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn aliases_build_and_run_across_packages() {
    let dir = test_dir("build");
    let source = "import std::convert\n\
                  import std::fmt\n\
                  import std::str\n\
                  import math::generic\n\
                  open flow main\n\
                  {\n\
                      protect {\n\
                          mut xs: [u64] <- [1 as u64, 2 as u64, 3 as u64]\n\
                          sum <- generic::sum<u64>(xs)\n\
                          prefix <- str::concat(\"sum=\", \"\")\n\
                          print(fmt::line(str::concat(prefix, u64_to_bytes(sum))))\n\
                      }\n\
                  }\n";
    fs::write(dir.join("main.loi"), source).expect("source should be writable");
    let output = Command::new(loi())
        .args([
            "build",
            "main.loi",
            "-o",
            "app",
            "--lib-dir",
            repo_lib().to_str().unwrap(),
        ])
        .current_dir(&dir)
        .output()
        .expect("loi should run");
    assert!(
        output.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(dir.join("app"))
        .output()
        .expect("built program should run");
    assert_eq!(String::from_utf8_lossy(&run.stdout), "sum=6\n");
    let _ = fs::remove_dir_all(&dir);
}
