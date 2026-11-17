use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn loi() -> &'static str {
    env!("CARGO_BIN_EXE_loi")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn test_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from("/tmp/opencode").join(format!("loi-lib-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("test directory should be creatable");
    dir
}

fn has_c_compiler() -> bool {
    Command::new("cc")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn write_program(dir: &Path, imports: &[&str], body: &str) -> PathBuf {
    let mut source = String::new();
    for name in imports {
        source.push_str(&format!(
            "import \"{}\"\n",
            repo_root().join("lib").join(name).display()
        ));
    }
    source.push_str(body);
    let path = dir.join("main.loi");
    fs::write(&path, source).expect("program should be writable");
    path
}

fn check(path: &Path) -> Output {
    Command::new(loi())
        .arg("check")
        .arg(path)
        .output()
        .expect("loi should run")
}

/// Check, build with both library shims, and run. Returns `None` when no C
/// compiler is available, so the pure-Loi assertions still run everywhere.
fn build_and_run(name: &str, imports: &[&str], body: &str) -> Option<Output> {
    let dir = test_dir(name);
    let program = write_program(&dir, imports, body);

    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    if !has_c_compiler() {
        let _ = fs::remove_dir_all(&dir);
        return None;
    }

    let executable = dir.join("program");
    let mut build = Command::new(loi());
    build.arg("build").arg(&program).arg("-o").arg(&executable);
    for shim in ["os_shim.c", "net_shim.c"] {
        build
            .arg("--c-source")
            .arg(repo_root().join("lib").join(shim));
    }
    let built = build.output().expect("loi build should run");
    assert!(
        built.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let run = Command::new(&executable)
        .output()
        .expect("built program should run");
    let _ = fs::remove_dir_all(&dir);
    Some(run)
}

fn assert_ok(name: &str, imports: &[&str], body: &str, stdout: &str) {
    if let Some(run) = build_and_run(name, imports, body) {
        assert_eq!(
            run.status.code(),
            Some(0),
            "{} failed: {}",
            name,
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&run.stdout), stdout);
    }
}

#[test]
fn every_library_source_checks_cleanly() {
    for name in ["std.loi", "math.loi", "ctype.loi", "os.loi", "net.loi"] {
        let path = repo_root().join("lib").join(name);
        let output = check(&path);
        assert!(
            output.status.success(),
            "{name} does not check: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn library_shims_compile_as_strict_c11() {
    if !has_c_compiler() {
        eprintln!("skipping library_shims_compile_as_strict_c11: cc not found");
        return;
    }
    for name in ["os_shim.c", "net_shim.c"] {
        let output = Command::new("cc")
            .args(["-std=c11", "-Wall", "-Wextra", "-fsyntax-only"])
            .arg(repo_root().join("lib").join(name))
            .output()
            .expect("cc should run");
        assert!(
            output.status.success(),
            "{name} does not compile: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn math_library_uses_generics_comptime_and_intrinsics() {
    assert_ok(
        "math",
        &["math.loi"],
        r#"
open flow main
    => i32
{
    if MATH_TAU != 6.283185307179586 { => 1 }
    if comptime math_factorial(5) != 120 { => 2 }
    if math_min<i32>(3, 5) != 3 { => 3 }
    if math_max<i32>(3, 5) != 5 { => 4 }
    if math_clamp<i32>(10, 0, 8) != 8 { => 5 }
    numbers: [i32] <- [i32: 1, 2, 3]
    if math_sum<i32>(numbers) != 6 { => 6 }
    fractions: [f64] <- [f64: 0.5, 1.5]
    if not math_near(math_sum<f64>(fractions), 2.0, 0.00001) { => 7 }
    if not math_near(math_lerp(0.0, 10.0, 0.25), 2.5, 0.00001) { => 8 }
    if not math_near(math_smoothstep(0.0, 1.0, 0.5), 0.5, 0.00001) { => 9 }
    if not math_near(math_hypot(3.0, 4.0), 5.0, 0.00001) { => 10 }
    if math_gcd_u64(12, 18) != 6 { => 11 }
    if math_pow_u64(2, 10) != 1024 { => 12 }
    if math_sign_i32(-7) != -1 or math_sign_i32(0) != 0 { => 13 }
    if not math_near(math_deg_to_rad(180.0), MATH_PI, 0.00001) { => 14 }
    if not math_near(math_rad_to_deg(MATH_PI), 180.0, 0.00001) { => 15 }
    if not math_near(math_dot_f64(fractions, fractions), 2.5, 0.00001) { => 16 }
    printf("math ok tau=%.3f\n", MATH_TAU)
    => 0
}
"#,
        "math ok tau=6.283\n",
    );
}

#[test]
fn ctype_library_classifies_and_parses_ascii_bytes() {
    assert_ok(
        "ctype",
        &["ctype.loi"],
        r#"
open flow main
    => i32
{
    if not ctype_is_digit(53) or ctype_is_digit(65) { => 1 }
    if not ctype_is_alpha(65) or not ctype_is_lower(122) { => 2 }
    if not ctype_is_alnum(48) or ctype_is_alnum(32) { => 3 }
    if not ctype_is_hex_digit(102) or ctype_is_hex_digit(103) { => 4 }
    if not ctype_is_space(9) or ctype_is_space(48) { => 5 }
    if not ctype_is_ascii(127) or ctype_is_ascii(128) { => 6 }
    if ctype_to_lower(65) != 97 or ctype_to_upper(97) != 65 { => 7 }
    if ctype_hex_value(57) != 9 or ctype_hex_value(97) != 10 { => 8 }
    if ctype_hex_value(71) != -1 { => 9 }
    parsed <- ctype_parse_u64("123abc")
    if not parsed.ok or parsed.value != 123 or parsed.consumed != 3 { => 10 }
    empty <- ctype_parse_u64("abc")
    if empty.ok or empty.consumed != 0 { => 11 }
    if ctype_count_digits("a1b22c") != 3 { => 12 }
    if not ctype_equals_ignore_case("Loi", "lOI") { => 13 }
    if ctype_equals_ignore_case("loi", "loi!") { => 14 }
    print("ctype ok\n")
    => 0
}
"#,
        "ctype ok\n",
    );
}

#[test]
fn os_library_reads_process_environment() {
    assert_ok(
        "os",
        &["os.loi"],
        r#"
open flow main
    => i32
{
    if os_pid() <= 0 { => 1 }
    if not os_has_env("PATH") { => 2 }
    if os_has_env("LOI_TEST_MISSING_VARIABLE") { => 3 }
    if os_env_i32("LOI_TEST_MISSING_VARIABLE", 17) != 17 { => 4 }
    if not os_sleep(0) or os_sleep(-1) { => 5 }
    if os_unix_time() <= 0 { => 6 }
    if os_errno() < 0 { => 7 }
    print("os ok\n")
    => 0
}
"#,
        "os ok\n",
    );
}

#[test]
fn net_library_parses_addresses_and_rejects_bad_connections() {
    assert_ok(
        "net",
        &["net.loi"],
        r#"
open flow main
    => i32
{
    if net_ipv4(127, 0, 0, 1) != 2130706433 { => 1 }
    if net_ipv4_octet(2130706433, 0) != 127 or net_ipv4_octet(2130706433, 3) != 1 { => 2 }
    if net_ipv4_octet(2130706433, 4) != 0 { => 3 }
    if not net_port_is_valid(80) or net_port_is_valid(0) or net_port_is_valid(65536) { => 4 }
    parsed <- net_parse_ipv4("192.168.1.2")
    if not parsed.ok or parsed.addr != net_ipv4(192, 168, 1, 2) { => 5 }
    broken <- net_parse_ipv4("192.168.1")
    if broken.ok { => 6 }
    large <- net_parse_ipv4("256.0.0.1")
    if large.ok { => 7 }
    if net_tcp_connect("127.0.0.1", 0) != -1 { => 8 }
    if net_tcp_connect("not-an-ip", 80) != -1 { => 9 }
    if net_send(-1, "hello") { => 10 }
    net_close(-1)
    protect {
        mut buffer: [u8]
        alloc(-> buffer, u8, 4)
        if loi_net_recv(-1, <-> buffer) != -1 { => 11 }
        if net_recv(-1, <-> buffer) != 0 { => 12 }
    }
    print("net ok\n")
    => 0
}
"#,
        "net ok\n",
    );
}

#[test]
fn all_libraries_can_be_imported_together() {
    assert_ok(
        "all-libraries",
        &["std.loi", "math.loi", "ctype.loi", "os.loi", "net.loi"],
        r#"
open flow main
    => i32
{
    if min_i32(3, 5) != 3 or not str_eq("loi", "loi") { => 1 }
    if str_find("hello", "ll") != 2 { => 2 }
    if math_sum<i32>([i32: 1, 2, 3]) != 6 { => 3 }
    if not ctype_is_alnum(90) { => 4 }
    if os_pid() <= 0 { => 5 }
    if not net_port_is_valid(443) { => 6 }
    mut ok: i32 <- 0
    if os_pid() > 0 {
        ok = 1
    }
    printf("all ok %d\n", ok)
    => 0
}
"#,
        "all ok 1\n",
    );
}
