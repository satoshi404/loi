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

fn write_source(dir: &Path, name: &str, source: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, source).expect("source file should be writable");
    path
}

/// Compile the emitted C in the three requested modes. ASan+UBSan binaries are
/// built but not executed: this PRoot container cannot map ASan's shadow
/// memory. UBSan-only binaries are compiled and executed by the callers.
fn compile_c_variants(
    dir: &Path,
    program: &Path,
    extra_sources: &[PathBuf],
) -> Option<(PathBuf, PathBuf)> {
    if !has_c_compiler() {
        return None;
    }
    let c = dir.join("main.c");
    let generated = Command::new(loi())
        .arg("gen")
        .arg(program)
        .arg("-o")
        .arg(&c)
        .output()
        .expect("loi gen should run");
    assert!(
        generated.status.success(),
        "loi gen failed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );

    let strict = dir.join("strict");
    let strict_output = Command::new("cc")
        .args(["-std=c11", "-pedantic-errors", "-Wall", "-Wextra"])
        .arg(&c)
        .args(extra_sources)
        .arg("-o")
        .arg(&strict)
        .output()
        .expect("cc should run");
    assert!(
        strict_output.status.success() && strict_output.stderr.is_empty(),
        "strict C11 build failed: {}",
        String::from_utf8_lossy(&strict_output.stderr)
    );

    let ubsan = dir.join("ubsan");
    let ubsan_output = Command::new("cc")
        .args([
            "-std=c11",
            "-fsanitize=undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ])
        .arg(&c)
        .args(extra_sources)
        .arg("-o")
        .arg(&ubsan)
        .output()
        .expect("cc should run");
    assert!(
        ubsan_output.status.success(),
        "UBSan build failed: {}",
        String::from_utf8_lossy(&ubsan_output.stderr)
    );

    let asan = dir.join("asan-ubsan");
    let asan_output = Command::new("cc")
        .args([
            "-std=c11",
            "-fsanitize=address,undefined",
            "-fno-omit-frame-pointer",
            "-g",
        ])
        .arg(&c)
        .args(extra_sources)
        .arg("-o")
        .arg(&asan)
        .output()
        .expect("cc should run");
    assert!(
        asan_output.status.success(),
        "ASan+UBSan build failed: {}",
        String::from_utf8_lossy(&asan_output.stderr)
    );
    Some((strict, ubsan))
}

fn assert_ok_with_c_variants(
    name: &str,
    imports: &[&str],
    body: &str,
    extra_sources: &[PathBuf],
    stdout: &str,
) {
    let dir = test_dir(name);
    let program = write_program(&dir, imports, body);
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );

    if let Some((strict, ubsan)) = compile_c_variants(&dir, &program, extra_sources) {
        for executable in [strict, ubsan] {
            let run = Command::new(&executable)
                .output()
                .expect("C program should run");
            assert_eq!(
                run.status.code(),
                Some(0),
                "{name} failed: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&run.stdout), stdout);
            assert!(
                run.stderr.is_empty(),
                "{name} wrote stderr: {}",
                String::from_utf8_lossy(&run.stderr)
            );
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

fn assert_runtime_error(name: &str, imports: &[&str], body: &str, stderr: &str) {
    let dir = test_dir(name);
    let program = write_program(&dir, imports, body);
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    compile_c_variants(&dir, &program, &[]);

    let executable = dir.join("program");
    let built = Command::new(loi())
        .arg("build")
        .arg(&program)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("loi build should run");
    assert!(
        built.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let run = Command::new(&executable)
        .output()
        .expect("built program should run");
    assert_eq!(run.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&run.stderr), stderr);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn every_library_source_checks_cleanly() {
    // Allocator-backed libraries are checked through the programs below: the
    // checker only accepts their `alloc` calls when a caller uses `protect`.
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
fn vec_library_manages_growth_cloneable_elements_and_ordering() {
    assert_ok_with_c_variants(
        "vec-core",
        &["vec.loi", "slice.loi"],
        r#"
shape Point [
    x: i32
    y: i32
]

open flow main
    => i32
{
    protect {
        mut xs: Vec<i32>
        vec_new<i32>(-> xs)
        if vec_len<i32>(xs) != 0 or vec_capacity<i32>(xs) != 0 { => 1 }
        if not vec_is_empty<i32>(xs) { => 2 }

        vec_push<i32>(<-> xs, 10)
        vec_push<i32>(<-> xs, 20)
        vec_push<i32>(<-> xs, 30)
        if vec_len<i32>(xs) != 3 or vec_capacity<i32>(xs) != 4 { => 3 }
        if vec_is_empty<i32>(xs) { => 4 }
        if vec_get<i32>(xs, 1) != 20 { => 5 }

        vec_set<i32>(<-> xs, 1, 21)
        if vec_get<i32>(xs, 1) != 21 { => 6 }
        if vec_find<i32>(xs, 30) != 2 or vec_find<i32>(xs, 99) != -1 { => 7 }

        popped <- vec_pop<i32>(<-> xs)
        if popped != 30 or vec_len<i32>(xs) != 2 { => 8 }
        vec_reverse<i32>(<-> xs)
        if vec_get<i32>(xs, 0) != 21 or vec_get<i32>(xs, 1) != 10 { => 9 }

        view <- vec_as_slice<i32>(xs)
        if slice_len<i32>(view) != 2 or slice_get<i32>(view, 1) != 10 { => 10 }

        vec_reserve<i32>(<-> xs, 8)
        if vec_capacity<i32>(xs) != 8 or vec_get<i32>(xs, 0) != 21 { => 11 }
        vec_clear<i32>(<-> xs)
        if vec_len<i32>(xs) != 0 or not vec_is_empty<i32>(xs) { => 12 }

        source: [i32] <- [i32: 4, 5, 6]
        mut made: Vec<i32>
        vec_from_slice<i32>(source, -> made)
        if vec_len<i32>(made) != 3 or vec_capacity<i32>(made) != 3 { => 13 }
        if vec_get<i32>(made, 2) != 6 { => 14 }

        unsorted: [i32] <- [i32: 5, -2, 9, 0, 5, 1]
        mut sorted: Vec<i32>
        vec_from_slice<i32>(unsorted, -> sorted)
        vec_sort<i32>(<-> sorted)
        if vec_get<i32>(sorted, 0) != -2 or vec_get<i32>(sorted, 1) != 0 { => 15 }
        if vec_get<i32>(sorted, 2) != 1 or vec_get<i32>(sorted, 3) != 5 { => 16 }
        if vec_get<i32>(sorted, 4) != 5 or vec_get<i32>(sorted, 5) != 9 { => 17 }
        if vec_binary_search<i32>(sorted, 5) != 3 { => 18 }
        if vec_binary_search<i32>(sorted, 4) != -1 { => 19 }

        mut points: Vec<Point>
        vec_new<Point>(-> points)
        first <- Point[ x: 1, y: 2 ]
        second <- Point[ x: 3, y: 4 ]
        vec_push<Point>(<-> points, first)
        vec_push<Point>(<-> points, second)
        point <- vec_get<Point>(points, 1)
        if point.x != 3 or point.y != 4 { => 20 }
        replacement <- Point[ x: 9, y: 8 ]
        vec_set<Point>(<-> points, 1, replacement)
        vec_reverse<Point>(<-> points)
        if vec_get<Point>(points, 0).x != 9 or vec_get<Point>(points, 1).x != 1 { => 21 }
        point_view <- vec_as_slice<Point>(points)
        if slice_len<Point>(point_view) != 2 { => 22 }
        mut copied_points: Vec<Point>
        vec_from_slice<Point>(point_view, -> copied_points)
        vec_reserve<Point>(<-> copied_points, 5)
        if vec_capacity<Point>(copied_points) != 5 { => 23 }
        last_point <- vec_pop<Point>(<-> copied_points)
        if last_point.x != 1 or vec_len<Point>(copied_points) != 1 { => 24 }
        vec_clear<Point>(<-> copied_points)
        if not vec_is_empty<Point>(copied_points) { => 25 }

        print("vec ok\n")
    }
    => 0
}
"#,
        &[],
        "vec ok\n",
    );
}

#[test]
fn vec_new_does_not_require_a_protect_region() {
    assert_ok(
        "vec-new-no-protect",
        &["vec.loi"],
        r#"
open flow main
    => i32
{
    mut xs: Vec<i32>
    vec_new<i32>(-> xs)
    if vec_len<i32>(xs) != 0 or vec_capacity<i32>(xs) != 0 { => 1 }
    print("empty vector ok\n")
    => 0
}
"#,
        "empty vector ok\n",
    );
}

#[test]
fn vec_find_reports_the_eq_bound_at_the_call_site() {
    let dir = test_dir("vec-find-bound");
    let program = write_program(
        &dir,
        &["vec.loi"],
        r#"
shape Point [ x: i32 ]

open flow main
    => i32
{
    mut xs: Vec<Point>
    vec_new<Point>(-> xs)
    p <- Point[ x: 1 ]
    found <- vec_find<Point>(xs, p)
    => 0
}
"#,
    );
    let checked = check(&program);
    assert!(
        !checked.status.success(),
        "an unbounded element type must be rejected"
    );
    let stderr = String::from_utf8_lossy(&checked.stderr);
    assert!(
        stderr.contains("does not satisfy the 'Eq' bound"),
        "{stderr}"
    );
    assert!(stderr.contains("vec_find"), "{stderr}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn vec_find_accepts_non_numeric_eq_types() {
    assert_ok(
        "vec-find-bool",
        &["vec.loi"],
        r#"
open flow main
    => i32
{
    protect {
        mut xs: Vec<bool>
        vec_new<bool>(-> xs)
        vec_push<bool>(<-> xs, false)
        vec_push<bool>(<-> xs, true)
        if vec_find<bool>(xs, true) != 1 { => 1 }
    }
    print("vec find bool ok\n")
    => 0
}
"#,
        "vec find bool ok\n",
    );
}

#[test]
fn vec_library_covers_u8_and_f64_instantiations() {
    assert_ok_with_c_variants(
        "vec-numeric",
        &["vec.loi"],
        r#"
open flow main
    => i32
{
    protect {
        mut bytes: Vec<u8>
        vec_new<u8>(-> bytes)
        vec_push<u8>(<-> bytes, 3)
        vec_push<u8>(<-> bytes, 1)
        vec_push<u8>(<-> bytes, 2)
        vec_sort<u8>(<-> bytes)
        if vec_get<u8>(bytes, 0) != 1 or vec_get<u8>(bytes, 2) != 3 { => 1 }
        if vec_binary_search<u8>(bytes, 2) != 1 { => 2 }
        if vec_find<u8>(bytes, 9) != -1 { => 3 }

        mut floats: Vec<f64>
        vec_new<f64>(-> floats)
        vec_push<f64>(<-> floats, 2.5)
        vec_push<f64>(<-> floats, 1.5)
        vec_sort<f64>(<-> floats)
        if vec_get<f64>(floats, 0) != 1.5 or vec_get<f64>(floats, 1) != 2.5 { => 4 }
        if vec_binary_search<f64>(floats, 2.5) != 1 { => 5 }
        popped <- vec_pop<f64>(<-> floats)
        if popped != 2.5 { => 6 }

        print("vec numeric ok\n")
    }
    => 0
}
"#,
        &[],
        "vec numeric ok\n",
    );
}

#[test]
fn vec_library_stores_slice_elements_with_independent_storage() {
    assert_ok_with_c_variants(
        "vec-slice-elements",
        &["vec.loi"],
        r#"
open flow main
    => i32
{
    protect {
        mut rows: Vec<[u8]>
        vec_new<[u8]>(-> rows)
        mut first: [u8]
        alloc(-> first, u8, 2)
        first[0] = 97
        first[1] = 98
        vec_push<[u8]>(<-> rows, first)
        first[0] = 90
        mut got: [u8] <- vec_get<[u8]>(rows, 0)
        if got[0] != 97 or got[1] != 98 { => 1 }
        got[0] = 91
        mut again: [u8] <- vec_get<[u8]>(rows, 0)
        if again[0] != 97 { => 2 }
        print("vec slice elements ok\n")
    }
    => 0
}
"#,
        &[],
        "vec slice elements ok\n",
    );
}

#[test]
fn slice_library_covers_every_flow_and_cloneable_elements() {
    assert_ok_with_c_variants(
        "slice-core",
        &["slice.loi"],
        r#"
shape Point [
    x: i32
    y: i32
]

open flow main
    => i32
{
    protect {
        values: [i32] <- [i32: 4, 1, 3, 2]
        if slice_len<i32>(values) != 4 or slice_is_empty<i32>(values) { => 1 }
        if slice_get<i32>(values, 2) != 3 { => 2 }
        if slice_first<i32>(values) != 4 or slice_last<i32>(values) != 2 { => 3 }
        if slice_find<i32>(values, 3) != 2 or slice_find<i32>(values, 8) != -1 { => 4 }

        cloned <- slice_clone<i32>(values)
        if slice_len<i32>(cloned) != 4 or slice_get<i32>(cloned, 1) != 1 { => 5 }
        mut copied: [i32]
        slice_copy<i32>(values, -> copied)
        if slice_len<i32>(copied) != 4 or slice_get<i32>(copied, 3) != 2 { => 6 }

        mut reversed: [i32] <- [i32: 4, 1, 3, 2]
        slice_reverse<i32>(<-> reversed)
        if reversed[0] != 2 or reversed[1] != 3 or reversed[2] != 1 or reversed[3] != 4 { => 7 }

        empty: [i32] <- [i32:]
        if not slice_is_empty<i32>(empty) { => 8 }
        empty_copy <- slice_clone<i32>(empty)
        if slice_len<i32>(empty_copy) != 0 { => 9 }

        bytes: [u8] <- [u8: 3, 1, 2]
        if slice_get<u8>(bytes, 0) != 3 or slice_find<u8>(bytes, 2) != 2 { => 10 }
        mut reversed_bytes: [u8] <- [u8: 3, 1, 2]
        slice_reverse<u8>(<-> reversed_bytes)
        if reversed_bytes[0] != 2 or reversed_bytes[2] != 3 { => 11 }

        fractions: [f64] <- [f64: 1.5, 2.5, 3.5]
        if slice_get<f64>(fractions, 1) != 2.5 or slice_find<f64>(fractions, 3.5) != 2 { => 12 }
        mut reversed_fractions: [f64] <- [f64: 1.5, 2.5, 3.5]
        slice_reverse<f64>(<-> reversed_fractions)
        if reversed_fractions[0] != 3.5 or reversed_fractions[2] != 1.5 { => 13 }

        points: [Point] <- [Point[ x: 1, y: 2 ], Point[ x: 3, y: 4 ]]
        if slice_get<Point>(points, 1).x != 3 { => 14 }
        cloned_points <- slice_clone<Point>(points)
        if slice_len<Point>(cloned_points) != 2 { => 15 }
        mut copied_points: [Point]
        slice_copy<Point>(points, -> copied_points)
        if slice_get<Point>(copied_points, 0).y != 2 { => 16 }
        mut reversed_points: [Point] <- [Point[ x: 1, y: 2 ], Point[ x: 3, y: 4 ]]
        slice_reverse<Point>(<-> reversed_points)
        if slice_get<Point>(reversed_points, 0).x != 3 { => 17 }

        if not slice_equals_u8("abc", "abc") or slice_equals_u8("abc", "abd") { => 18 }
        print("slice ok\n")
    }
    => 0
}
"#,
        &[],
        "slice ok\n",
    );
}

#[test]
fn vector_and_slice_errors_name_the_flow() {
    assert_runtime_error(
        "vec-get-error",
        &["vec.loi"],
        r#"
open flow main
    => i32
{
    protect {
        values <- vec_from_slice_error_input()
        bad <- vec_get<i32>(values, 7)
        if bad == 0 { print("unreachable\n") }
    }
    => 0
}

flow vec_from_slice_error_input
    => Vec<i32>
{
    mut made: Vec<i32>
    vec_new<i32>(-> made)
    vec_push<i32>(<-> made, 1)
    vec_push<i32>(<-> made, 2)
    => give(made)
}
"#,
        "vec_get: index 7 out of range (len 2)\n",
    );

    assert_runtime_error(
        "vec-set-error",
        &["vec.loi"],
        r#"
open flow main
    => i32
{
    protect {
        mut values: Vec<i32>
        vec_new<i32>(-> values)
        vec_push<i32>(<-> values, 1)
        vec_set<i32>(<-> values, 7, 9)
        print("unreachable\n")
    }
    => 0
}
"#,
        "vec_set: index 7 out of range (len 1)\n",
    );

    assert_runtime_error(
        "vec-pop-error",
        &["vec.loi"],
        r#"
open flow main
    => i32
{
    protect {
        mut values: Vec<i32>
        vec_new<i32>(-> values)
        popped <- vec_pop<i32>(<-> values)
        if popped == 0 { print("unreachable\n") }
    }
    => 0
}
"#,
        "vec_pop: vector is empty (len 0)\n",
    );

    assert_runtime_error(
        "slice-get-error",
        &["slice.loi"],
        r#"
open flow main
    => i32
{
    protect {
        values: [i32] <- [i32: 1, 2]
        bad <- slice_get<i32>(values, 7)
        if bad == 0 { print("unreachable\n") }
    }
    => 0
}
"#,
        "slice_get: index 7 out of range (len 2)\n",
    );

    assert_runtime_error(
        "slice-first-error",
        &["slice.loi"],
        r#"
open flow main
    => i32
{
    protect {
        empty: [i32] <- [i32:]
        bad <- slice_first<i32>(empty)
        if bad == 0 { print("unreachable\n") }
    }
    => 0
}
"#,
        "slice_first: slice is empty (len 0)\n",
    );

    assert_runtime_error(
        "slice-last-error",
        &["slice.loi"],
        r#"
open flow main
    => i32
{
    protect {
        empty: [i32] <- [i32:]
        bad <- slice_last<i32>(empty)
        if bad == 0 { print("unreachable\n") }
    }
    => 0
}
"#,
        "slice_last: slice is empty (len 0)\n",
    );
}

#[test]
fn duplicate_library_imports_are_deduplicated() {
    assert_ok_with_c_variants(
        "duplicate-imports",
        &["vec.loi", "vec.loi", "slice.loi", "slice.loi"],
        r#"
open flow main
    => i32
{
    protect {
        mut values: Vec<i32>
        vec_new<i32>(-> values)
        vec_push<i32>(<-> values, 3)
        view <- vec_as_slice<i32>(values)
        if slice_len<i32>(view) != 1 or vec_get<i32>(values, 0) != 3 { => 1 }
        print("duplicate imports ok\n")
    }
    => 0
}
"#,
        &[],
        "duplicate imports ok\n",
    );
}

#[test]
fn two_libraries_can_import_vec_and_slice() {
    let dir = test_dir("cross-library-imports");
    let vec_path = repo_root().join("lib").join("vec.loi");
    let slice_path = repo_root().join("lib").join("slice.loi");
    let a = write_source(
        &dir,
        "wrapper_a.loi",
        &format!(
            "import \"{}\"\n\nopen flow sum_i32\n    <- values: [i32]\n    => i32\n{{\n    mut total: i32 <- 0\n    for i in 0..values.len {{\n        total += values[i]\n    }}\n    => total\n}}\n",
            vec_path.display()
        ),
    );
    let b = write_source(
        &dir,
        "wrapper_b.loi",
        &format!(
            "import \"{}\"\n\nopen flow count_i32\n    <- values: [i32]\n    => u64\n{{\n    => slice_len<i32>(values)\n}}\n",
            slice_path.display()
        ),
    );
    let program = write_source(
        &dir,
        "main.loi",
        &format!(
            "import \"{}\"\nimport \"{}\"\n\nopen flow main\n    => i32\n{{\n    protect {{\n        mut values: Vec<i32>\n        vec_new<i32>(-> values)\n        vec_push<i32>(<-> values, 2)\n        vec_push<i32>(<-> values, 4)\n        view <- vec_as_slice<i32>(values)\n        if sum_i32(view) != 6 or count_i32(view) != 2 {{ => 1 }}\n        print(\"cross library ok\\n\")\n    }}\n    => 0\n}}\n",
            a.display(),
            b.display()
        ),
    );
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    if let Some((strict, ubsan)) = compile_c_variants(&dir, &program, &[]) {
        for executable in [strict, ubsan] {
            let run = Command::new(&executable)
                .output()
                .expect("C program should run");
            assert_eq!(
                run.status.code(),
                Some(0),
                "cross-library import failed: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&run.stdout), "cross library ok\n");
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

struct SeededRng(u64);

impl SeededRng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn next_i32(&mut self, low: i32, high: i32) -> i32 {
        low + (self.next_u64() % (high - low + 1) as u64) as i32
    }
}

fn loi_i32_slice(values: &[i32]) -> String {
    let items = values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[i32: {items}]")
}

#[test]
fn seeded_sort_and_find_match_known_vectors_and_naive_lists() {
    let mut rng = SeededRng(0x5eed_1234_5678_9abc);
    let mut cases: Vec<Vec<i32>> = vec![
        vec![],
        vec![7],
        vec![3, 3, 3, 3],
        vec![-5, 4, -3, 2, -1, 0],
        vec![9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
    ];
    for size in [1, 2, 5, 11, 23] {
        cases.push((0..size).map(|_| rng.next_i32(-50, 50)).collect::<Vec<_>>());
    }

    let mut body = String::from("\nopen flow main\n    => i32\n{\n    protect {\n");
    let mut expected = String::new();
    for (index, values) in cases.iter().enumerate() {
        let literal = loi_i32_slice(values);
        let length = values.len();
        let guard = 100 + index;
        body.push_str(&format!(
            "        source{index}: [i32] <- {literal}\n\
             \x20       mut vector{index}: Vec<i32>\n\
             \x20       vec_from_slice<i32>(source{index}, -> vector{index})\n\
             \x20       vec_sort<i32>(<-> vector{index})\n\
             \x20       sorted{index} <- vec_as_slice<i32>(vector{index})\n\
             \x20       if slice_len<i32>(sorted{index}) != {length} {{ => {guard} }}\n\
             \x20       print(\"sort{index}:\")\n"
        ));
        for at in 0..values.len() {
            body.push_str(&format!(
                "        printf(\" %d\", slice_get<i32>(sorted{index}, {at}))\n"
            ));
        }
        body.push_str("        print(\"\\n\")\n");

        let mut sorted = values.clone();
        sorted.sort_unstable();
        let joined = sorted
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        if joined.is_empty() {
            expected.push_str(&format!("sort{index}:\n"));
        } else {
            expected.push_str(&format!("sort{index}: {joined}\n"));
        }
    }

    for (index, values) in cases.iter().enumerate() {
        let mut sorted_values = values.clone();
        sorted_values.sort_unstable();
        let needle = if index % 3 == 0 {
            sorted_values.first().copied().unwrap_or(0)
        } else {
            rng.next_i32(-50, 50)
        };
        let expected_index = sorted_values.iter().position(|value| *value == needle);
        body.push_str(&format!(
            "        found{index} <- vec_find<i32>(vector{index}, {needle})\n\
             \x20       printf(\"find{index}:%d\\n\", found{index})\n"
        ));
        expected.push_str(&format!(
            "find{index}:{}\n",
            expected_index.map_or(-1, |at| at as i32)
        ));
    }

    body.push_str("        print(\"properties ok\\n\")\n    }\n    => 0\n}\n");
    expected.push_str("properties ok\n");

    assert_ok_with_c_variants(
        "seeded-properties",
        &["vec.loi", "slice.loi"],
        &body,
        &[],
        &expected,
    );
}

#[test]
fn seeded_sort_matches_c_qsort() {
    let shim = r#"
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

typedef struct {
    void* data;
    size_t len;
} LoiSlice;

static int compare_i32(const void* left, const void* right) {
    int32_t a = *(const int32_t*)left;
    int32_t b = *(const int32_t*)right;
    if (a < b) {
        return -1;
    }
    if (a > b) {
        return 1;
    }
    return 0;
}

void loi_qsort_i32_ref(LoiSlice* values) {
    if (values != NULL && values->data != NULL && values->len > 1) {
        qsort(values->data, values->len, sizeof(int32_t), compare_i32);
    }
}
"#;
    let mut rng = SeededRng(0xc0ff_ee01_2345_6789);
    let cases: Vec<Vec<i32>> = [0, 1, 2, 7, 19]
        .into_iter()
        .map(|size| (0..size).map(|_| rng.next_i32(-100, 100)).collect())
        .collect();

    let mut body = String::from(
        "\nextern flow loi_qsort_i32_ref\n    <-> values: [i32]\n\nopen flow main\n    => i32\n{\n    protect {\n",
    );
    for (index, values) in cases.iter().enumerate() {
        let literal = loi_i32_slice(values);
        body.push_str(&format!(
            "        source{index}: [i32] <- {literal}\n\
             \x20       mut vector{index}: Vec<i32>\n\
             \x20       vec_from_slice<i32>(source{index}, -> vector{index})\n\
             \x20       vec_sort<i32>(<-> vector{index})\n\
             \x20       sorted{index} <- vec_as_slice<i32>(vector{index})\n\
             \x20       mut reference{index}: [i32] <- {literal}\n\
             \x20       loi_qsort_i32_ref(<-> reference{index})\n\
             \x20       mut same{index}: bool <- true\n\
             \x20       if slice_len<i32>(sorted{index}) != reference{index}.len {{\n\
             \x20           same{index} = false\n\
             \x20       }}\n\
             \x20       if same{index} {{\n\
             \x20           for i in 0..reference{index}.len {{\n\
             \x20               if slice_get<i32>(sorted{index}, i) != reference{index}[i] {{\n\
             \x20                   same{index} = false\n\
             \x20               }}\n\
             \x20           }}\n\
             \x20       }}\n\
             \x20       if same{index} {{\n\
             \x20           print(\"qsort{index}:ok\\n\")\n\
             \x20       }} else {{\n\
             \x20           print(\"qsort{index}:bad\\n\")\n\
             \x20       }}\n"
        ));
    }
    body.push_str("        print(\"qsort ok\\n\")\n    }\n    => 0\n}\n");

    let dir = test_dir("qsort-reference");
    let shim_path = write_source(&dir, "qsort_shim.c", shim);
    let program = write_program(&dir, &["vec.loi", "slice.loi"], &body);
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let expected = "qsort0:ok\nqsort1:ok\nqsort2:ok\nqsort3:ok\nqsort4:ok\nqsort ok\n".to_string();
    if let Some((strict, ubsan)) = compile_c_variants(&dir, &program, &[shim_path]) {
        for executable in [strict, ubsan] {
            let run = Command::new(&executable)
                .output()
                .expect("C program should run");
            assert_eq!(
                run.status.code(),
                Some(0),
                "qsort comparison failed: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&run.stdout), expected);
            assert!(run.stderr.is_empty());
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn string_library_trims_concatenates_reverses_and_repeats_bytes() {
    assert_ok_with_c_variants(
        "string-core",
        &["std.loi", "string.loi"],
        r#"
open flow main
    => i32
{
    protect {
        trimmed <- str_trim("  hi\n\t")
        if not str_eq(trimmed, "hi") { => 1 }
        already <- str_trim("plain")
        if not str_eq(already, "plain") { => 2 }
        blank <- str_trim("   \n\t ")
        if str_len(blank) != 0 { => 3 }

        joined <- str_concat("foo", "bar")
        if not str_eq(joined, "foobar") { => 4 }
        empty_join <- str_concat("", "tail")
        if not str_eq(empty_join, "tail") { => 5 }

        reversed <- str_reverse("abcdef")
        if not str_eq(reversed, "fedcba") { => 6 }
        empty_reverse <- str_reverse("")
        if str_len(empty_reverse) != 0 { => 7 }

        repeated <- str_repeat("ab", 3)
        if not str_eq(repeated, "ababab") { => 8 }
        empty_repeat <- str_repeat("x", 0)
        if str_len(empty_repeat) != 0 { => 9 }
        single <- str_repeat("loi", 1)
        if not str_eq(single, "loi") { => 10 }

        if str_count("banana", 97) != 3 or str_count("banana", 120) != 0 { => 11 }
        if str_index_of("", "") != 0 or str_index_of("abc", "abcd") != -1 { => 12 }
        if str_index_of("banana", "n") != 2 or str_index_of("banana", "x") != -1 { => 13 }
        if not str_contains("banana", "n") or str_contains("banana", "x") { => 14 }
        if not str_contains("", "") { => 15 }

        print("string ok\n")
    }
    => 0
}
"#,
        &[],
        "string ok\n",
    );
}

#[test]
fn hash_library_matches_known_vectors() {
    assert_ok_with_c_variants(
        "hash-known",
        &["hash.loi"],
        r#"
open flow main
    => i32
{
    if HASH_OFFSET != 14695981039346656037 { => 1 }
    if HASH_PRIME != 1099511628211 { => 2 }
    if hash_bytes("") != HASH_OFFSET { => 3 }
    if hash_str("a") != 12638187200555641996 { => 4 }
    if hash_str("hello") != 11831194018420276491 { => 5 }
    if hash_u64(0) != 12161962213042174405 { => 6 }
    if hash_u64(1) != 9929646806074584996 { => 7 }
    if hash_u64(0x0102030405060708) != 895447315735140821 { => 8 }
    if hash_bytes_seeded("abc", 42) != 9828954460287139422 { => 9 }
    if hash_pair_u64(7, 9) != 12823680747875972811 { => 10 }
    if hash_u64_seeded(1, 42) != 3755842131058586507 { => 11 }
    if hash_u64_seeded(0x0102030405060708, 99) != 16797047285318259627 { => 12 }
    print("hash ok\n")
    => 0
}
"#,
        &[],
        "hash ok\n",
    );
}

fn loi_u8_slice(values: &[u8]) -> String {
    let items = values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[u8: {items}]")
}

const HASH_OFFSET: u64 = 0xcbf29ce484222325;
const HASH_PRIME: u64 = 0x100000001b3;

fn reference_hash_bytes(data: &[u8]) -> u64 {
    let mut hash = HASH_OFFSET;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(HASH_PRIME);
    }
    hash
}

fn reference_hash_u64(value: u64) -> u64 {
    reference_hash_bytes(&value.to_le_bytes())
}

fn reference_hash_seeded(data: &[u8], seed: u64) -> u64 {
    let mut hash = seed;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(HASH_PRIME);
    }
    hash
}

#[test]
fn seeded_hashes_match_the_fnv_reference() {
    let mut rng = SeededRng(0xf1a7_5eed_0bad_c0de);
    let mut body = String::from("\nopen flow main\n    => i32\n{\n");
    let mut expected = String::new();
    for index in 0..8 {
        let size = (index * 3) % 17;
        let bytes: Vec<u8> = (0..size).map(|_| rng.next_u64() as u8).collect();
        let seed = rng.next_u64();
        let value = rng.next_u64();
        body.push_str(&format!(
            "    data{index}: [u8] <- {literal}\n\
             \x20   printf(\"bytes{index}:%llu\\n\", hash_bytes(data{index}))\n\
             \x20   printf(\"seeded{index}:%llu\\n\", hash_bytes_seeded(data{index}, {seed}))\n\
             \x20   printf(\"u64_{index}:%llu\\n\", hash_u64({value}))\n\
             \x20   printf(\"seedu64_{index}:%llu\\n\", hash_u64_seeded({value}, {seed}))\n",
            literal = loi_u8_slice(&bytes)
        ));
        expected.push_str(&format!(
            "bytes{index}:{}\nseeded{index}:{}\nu64_{index}:{}\nseedu64_{index}:{}\n",
            reference_hash_bytes(&bytes),
            reference_hash_seeded(&bytes, seed),
            reference_hash_u64(value),
            reference_hash_seeded(&value.to_le_bytes(), seed)
        ));
    }
    body.push_str("    print(\"hash properties ok\\n\")\n    => 0\n}\n");
    expected.push_str("hash properties ok\n");

    assert_ok_with_c_variants("hash-properties", &["hash.loi"], &body, &[], &expected);
}

#[test]
fn string_and_hash_imports_work_twice_and_across_libraries() {
    assert_ok_with_c_variants(
        "string-hash-duplicate-imports",
        &["string.loi", "string.loi", "hash.loi", "hash.loi"],
        r#"
open flow main
    => i32
{
    protect {
        value <- str_trim("  loi  ")
        if str_count(value, 108) != 1 or hash_str(value) == 0 { => 1 }
        print("duplicate string imports ok\n")
    }
    => 0
}
"#,
        &[],
        "duplicate string imports ok\n",
    );

    let dir = test_dir("string-hash-cross-library");
    let string_path = repo_root().join("lib").join("string.loi");
    let hash_path = repo_root().join("lib").join("hash.loi");
    let a = write_source(
        &dir,
        "wrapper_a.loi",
        &format!(
            "import \"{}\"\n\nopen flow clean_word\n    <- text: [u8]\n    => [u8]\n{{\n    => str_trim(text)\n}}\n",
            string_path.display()
        ),
    );
    let b = write_source(
        &dir,
        "wrapper_b.loi",
        &format!(
            "import \"{}\"\n\nopen flow word_hash\n    <- text: [u8]\n    => u64\n{{\n    => hash_str(text)\n}}\n",
            hash_path.display()
        ),
    );
    let program = write_source(
        &dir,
        "main.loi",
        &format!(
            "import \"{}\"\nimport \"{}\"\n\nopen flow main\n    => i32\n{{\n    protect {{\n        word <- clean_word(\"  hash  \")\n        if word_hash(word) == 0 {{ => 1 }}\n        print(\"cross string imports ok\\n\")\n    }}\n    => 0\n}}\n",
            a.display(),
            b.display()
        ),
    );
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    if let Some((strict, ubsan)) = compile_c_variants(&dir, &program, &[]) {
        for executable in [strict, ubsan] {
            let run = Command::new(&executable)
                .output()
                .expect("C program should run");
            assert_eq!(
                run.status.code(),
                Some(0),
                "cross-library string import failed: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&run.stdout),
                "cross string imports ok\n"
            );
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn str_repeat_rejects_an_overflowing_count() {
    assert_runtime_error(
        "str-repeat-overflow",
        &["string.loi"],
        r#"
open flow main
    => i32
{
    protect {
        repeated <- str_repeat("ab", 18446744073709551615)
        if repeated.len == 0 { print("unreachable\n") }
    }
    => 0
}
"#,
        "str_repeat: count is too large\n",
    );
}

#[test]
fn crc32_library_matches_known_vectors_and_chunked_updates() {
    assert_ok_with_c_variants(
        "crc32-known",
        &["crc32.loi"],
        r#"
open flow main
    => i32
{
    if CRC32_POLY != 3988292384 or CRC32_INIT != 4294967295 { => 1 }
    if CRC32_XOROUT != 4294967295 { => 2 }
    if crc32_bytes("") != 0 { => 3 }
    if crc32_bytes("a") != 3904355907 { => 4 }
    if crc32_bytes("hello") != 907060870 { => 5 }
    if crc32_bytes("123456789") != 3421780262 { => 6 }
    if CRC32_TABLE[0] >> 32 != 498536548 or CRC32_TABLE[7] >> 32 != 3183342108 { => 7 }
    if CRC32_TABLE.len != 8 { => 9 }

    first <- crc32_update(CRC32_INIT, "1234")
    chunked <- crc32_update(first, "56789") ^ CRC32_XOROUT
    if chunked != 3421780262 { => 10 }
    empty_first <- crc32_update(CRC32_INIT, "")
    empty_final <- crc32_update(empty_first, "") ^ CRC32_XOROUT
    if empty_final != 0 { => 11 }

    zlib_first <- crc32_extend(0, "1234")
    zlib_second <- crc32_extend(zlib_first, "56789")
    if zlib_second != 3421780262 { => 12 }
    if crc32_extend(zlib_second, "") != zlib_second { => 13 }
    if crc32_extend(0, "123456789") != 3421780262 { => 14 }

    print("crc32 ok\n")
    => 0
}
"#,
        &[],
        "crc32 ok\n",
    );
}

const CRC32_POLY: u32 = 0xedb8_8320;

fn reference_crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ CRC32_POLY
            } else {
                crc >> 1
            };
        }
    }
    crc ^ u32::MAX
}

#[test]
fn seeded_crc32_matches_rust_and_c_bitwise_references() {
    let shim = r#"
#include <stddef.h>
#include <stdint.h>

typedef struct {
    void* data;
    size_t len;
} LoiSlice;

uint32_t loi_crc32_ref(LoiSlice* values) {
    if (values == NULL || values->data == NULL) {
        return 0;
    }
    const uint8_t* bytes = (const uint8_t*)values->data;
    uint32_t crc = UINT32_MAX;
    for (size_t i = 0; i < values->len; ++i) {
        crc ^= (uint32_t)bytes[i];
        for (int bit = 0; bit < 8; ++bit) {
            crc = (crc & 1u) != 0u ? (crc >> 1) ^ UINT32_C(0xedb88320) : (crc >> 1);
        }
    }
    return crc ^ UINT32_MAX;
}
"#;
    let mut rng = SeededRng(0x8c32_5eed_c0ff_ee01);
    let mut sizes = vec![0, 1, 2, 3, 15, 16, 17, 64];
    sizes.extend((0..6).map(|_| rng.next_u64() as usize % 129));
    let mut body = String::from(
        "\nextern flow loi_crc32_ref\n    <-> data: [u8]\n    => u32\n\nopen flow main\n    => i32\n{\n",
    );
    let mut expected = String::new();
    for (index, size) in sizes.iter().enumerate() {
        let bytes: Vec<u8> = (0..*size).map(|_| rng.next_u64() as u8).collect();
        let cut = size / 2;
        let first_literal = loi_u8_slice(&bytes[..cut]);
        let second_literal = loi_u8_slice(&bytes[cut..]);
        body.push_str(&format!(
            "    data{index}: [u8] <- {literal}\n\
             \x20   mut c_ref{index}: [u8] <- {literal}\n\
             \x20   printf(\"crc{index}:%llu\\n\", crc32_bytes(data{index}) as u64)\n\
             \x20   printf(\"ref{index}:%llu\\n\", loi_crc32_ref(<-> c_ref{index}) as u64)\n\
             \x20   first{index} <- crc32_update(4294967295, {first_literal})\n\
             \x20   printf(\"chunk{index}:%llu\\n\", (crc32_update(first{index}, {second_literal}) ^ 4294967295) as u64)\n",
            literal = loi_u8_slice(&bytes)
        ));
        let want = reference_crc32(&bytes);
        expected.push_str(&format!(
            "crc{index}:{want}\nref{index}:{want}\nchunk{index}:{want}\n"
        ));
    }
    body.push_str("    print(\"crc32 properties ok\\n\")\n    => 0\n}\n");
    expected.push_str("crc32 properties ok\n");

    let dir = test_dir("crc32-reference");
    let shim_path = write_source(&dir, "crc32_shim.c", shim);
    let program = write_program(&dir, &["crc32.loi"], &body);
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    if let Some((strict, ubsan)) = compile_c_variants(&dir, &program, &[shim_path]) {
        for executable in [strict, ubsan] {
            let run = Command::new(&executable)
                .output()
                .expect("C program should run");
            assert_eq!(
                run.status.code(),
                Some(0),
                "CRC32 comparison failed: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&run.stdout), expected);
            assert!(run.stderr.is_empty());
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn crc32_imports_work_twice_and_across_libraries() {
    assert_ok_with_c_variants(
        "crc32-duplicate-imports",
        &["crc32.loi", "crc32.loi"],
        r#"
open flow main
    => i32
{
    if crc32_bytes("loi") == 0 or CRC32_TABLE.len != 8 { => 1 }
    print("duplicate crc32 imports ok\n")
    => 0
}
"#,
        &[],
        "duplicate crc32 imports ok\n",
    );

    let dir = test_dir("crc32-cross-library");
    let crc_path = repo_root().join("lib").join("crc32.loi");
    let a = write_source(
        &dir,
        "wrapper_a.loi",
        &format!(
            "import \"{}\"\n\nopen flow digest\n    <- data: [u8]\n    => u32\n{{\n    => crc32_bytes(data)\n}}\n",
            crc_path.display()
        ),
    );
    let b = write_source(
        &dir,
        "wrapper_b.loi",
        &format!(
            "import \"{}\"\n\nopen flow digest_twice\n    <- data: [u8]\n    => u32\n{{\n    => crc32_update(crc32_update(CRC32_INIT, data), data)\n}}\n",
            crc_path.display()
        ),
    );
    let program = write_source(
        &dir,
        "main.loi",
        &format!(
            "import \"{}\"\nimport \"{}\"\n\nopen flow main\n    => i32\n{{\n    a <- digest(\"abc\")\n    b <- digest_twice(\"abc\")\n    if a == 0 or b == 0 or a == b {{ => 1 }}\n    print(\"cross crc32 imports ok\\n\")\n    => 0\n}}\n",
            a.display(),
            b.display()
        ),
    );
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    if let Some((strict, ubsan)) = compile_c_variants(&dir, &program, &[]) {
        for executable in [strict, ubsan] {
            let run = Command::new(&executable)
                .output()
                .expect("C program should run");
            assert_eq!(
                run.status.code(),
                Some(0),
                "cross-library crc32 import failed: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&run.stdout),
                "cross crc32 imports ok\n"
            );
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn map_int_library_covers_all_flows_and_growth() {
    assert_ok_with_c_variants(
        "map-int-core",
        &["map.loi"],
        r#"
shape Point [
    x: i32
    y: i32
]

open flow main
    => i32
{
    protect {
        mut ints: IntMap<i32>
        map_int_new<i32>(-> ints)
        if map_int_len<i32>(ints) != 0 or map_int_capacity<i32>(ints) != 8 { => 1 }
        map_int_set<i32>(<-> ints, 10, 100)
        map_int_set<i32>(<-> ints, 20, 200)
        map_int_set<i32>(<-> ints, 10, 111)
        if map_int_len<i32>(ints) != 2 { => 2 }
        if map_int_get<i32>(ints, 10) != 111 or map_int_get<i32>(ints, 20) != 200 { => 3 }
        if not map_int_contains<i32>(ints, 10) or map_int_contains<i32>(ints, 99) { => 4 }
        if map_int_get_or<i32>(ints, 10, -1) != 111 or map_int_get_or<i32>(ints, 99, -1) != -1 { => 101 }
        mut found: bool
        mut value: i32
        map_int_lookup<i32>(ints, 20, -1, -> found, -> value)
        if not found or value != 200 { => 102 }
        map_int_lookup<i32>(ints, 99, -1, -> found, -> value)
        if found or value != -1 { => 103 }

        if not map_int_remove<i32>(<-> ints, 10) { => 5 }
        if map_int_remove<i32>(<-> ints, 10) { => 6 }
        if map_int_contains<i32>(ints, 10) or map_int_len<i32>(ints) != 1 { => 7 }
        map_int_set<i32>(<-> ints, 20, 222)
        if map_int_get<i32>(ints, 20) != 222 { => 8 }

        mut i: u64 <- 0
        while i < 20 {
            map_int_set<i32>(<-> ints, i, i as i32 * 3)
            i += 1
        }
        if map_int_len<i32>(ints) != 21 or map_int_capacity<i32>(ints) != 32 { => 9 }
        if map_int_get<i32>(ints, 19) != 57 or map_int_get<i32>(ints, 0) != 0 { => 10 }
        if map_int_get<i32>(ints, 20) != 222 { => 11 }

        map_int_clear<i32>(<-> ints)
        if map_int_len<i32>(ints) != 0 or map_int_contains<i32>(ints, 19) { => 12 }

        mut points: IntMap<Point>
        map_int_new<Point>(-> points)
        first <- Point[ x: 1, y: 2 ]
        second <- Point[ x: 3, y: 4 ]
        map_int_set<Point>(<-> points, 1, first)
        map_int_set<Point>(<-> points, 2, second)
        if map_int_get<Point>(points, 1).x != 1 or map_int_get<Point>(points, 2).y != 4 { => 13 }
        replacement <- Point[ x: 9, y: 8 ]
        map_int_set<Point>(<-> points, 1, replacement)
        if map_int_get<Point>(points, 1).x != 9 { => 14 }
        if not map_int_remove<Point>(<-> points, 1) or map_int_len<Point>(points) != 1 { => 15 }
        map_int_clear<Point>(<-> points)
        if map_int_len<Point>(points) != 0 { => 16 }

        print("map int ok\n")
    }
    => 0
}
"#,
        &[],
        "map int ok\n",
    );
}

#[test]
fn map_str_library_covers_all_flows_and_growth() {
    assert_ok_with_c_variants(
        "map-str-core",
        &["map.loi"],
        r#"
shape Point [
    x: i32
    y: i32
]

open flow main
    => i32
{
    protect {
        mut words: StrMap<i32>
        map_str_new<i32>(-> words)
        if map_str_len<i32>(words) != 0 or map_str_capacity<i32>(words) != 8 { => 1 }
        map_str_set<i32>(<-> words, "alpha", 1)
        map_str_set<i32>(<-> words, "beta", 2)
        map_str_set<i32>(<-> words, "alpha", 3)
        if map_str_len<i32>(words) != 2 { => 2 }
        if map_str_get<i32>(words, "alpha") != 3 or map_str_get<i32>(words, "beta") != 2 { => 3 }
        if not map_str_contains<i32>(words, "alpha") or map_str_contains<i32>(words, "gamma") { => 4 }
        if map_str_get_or<i32>(words, "alpha", -1) != 3 or map_str_get_or<i32>(words, "gamma", -1) != -1 { => 101 }
        mut found: bool
        mut value: i32
        map_str_lookup<i32>(words, "beta", -1, -> found, -> value)
        if not found or value != 2 { => 102 }
        map_str_lookup<i32>(words, "gamma", -1, -> found, -> value)
        if found or value != -1 { => 103 }

        if map_str_contains<i32>(words, "") { => 5 }
        map_str_set<i32>(<-> words, "", 7)
        if map_str_get<i32>(words, "") != 7 { => 6 }
        if not map_str_remove<i32>(<-> words, "alpha") { => 7 }
        if map_str_contains<i32>(words, "alpha") or map_str_get<i32>(words, "beta") != 2 { => 8 }

        map_str_set<i32>(<-> words, "k0", 0)
        map_str_set<i32>(<-> words, "k1", 1)
        map_str_set<i32>(<-> words, "k2", 2)
        map_str_set<i32>(<-> words, "k3", 3)
        map_str_set<i32>(<-> words, "k4", 4)
        map_str_set<i32>(<-> words, "k5", 5)
        map_str_set<i32>(<-> words, "k6", 6)
        map_str_set<i32>(<-> words, "k7", 7)
        map_str_set<i32>(<-> words, "k8", 8)
        if map_str_capacity<i32>(words) != 16 { => 9 }
        if map_str_get<i32>(words, "k8") != 8 or map_str_get<i32>(words, "") != 7 { => 10 }

        long <- "0123456789012345678901234567890123456789012345678901234567890123456789"
        map_str_set<i32>(<-> words, long, 999)
        if map_str_get<i32>(words, long) != 999 { => 11 }
        if map_str_len<i32>(words) != 12 { => 12 }

        map_str_clear<i32>(<-> words)
        if map_str_len<i32>(words) != 0 or map_str_contains<i32>(words, long) { => 13 }
        map_str_set<i32>(<-> words, "fresh", 1)
        if map_str_get<i32>(words, "fresh") != 1 { => 14 }

        mut points: StrMap<Point>
        map_str_new<Point>(-> points)
        p <- Point[ x: 4, y: 5 ]
        map_str_set<Point>(<-> points, "point", p)
        if map_str_get<Point>(points, "point").y != 5 { => 15 }
        replacement <- Point[ x: 8, y: 9 ]
        map_str_set<Point>(<-> points, "point", replacement)
        if map_str_get<Point>(points, "point").x != 8 { => 16 }
        if not map_str_remove<Point>(<-> points, "point") or map_str_len<Point>(points) != 0 { => 17 }

        print("map str ok\n")
    }
    => 0
}
"#,
        &[],
        "map str ok\n",
    );
}

#[test]
fn map_missing_key_errors_name_the_flow() {
    assert_runtime_error(
        "map-int-get-error",
        &["map.loi"],
        r#"
open flow main
    => i32
{
    protect {
        mut m: IntMap<i32>
        map_int_new<i32>(-> m)
        bad <- map_int_get<i32>(m, 7)
        if bad == 0 { print("unreachable\n") }
    }
    => 0
}
"#,
        "map_int_get: missing key\n",
    );

    assert_runtime_error(
        "map-str-get-error",
        &["map.loi"],
        r#"
open flow main
    => i32
{
    protect {
        mut m: StrMap<i32>
        map_str_new<i32>(-> m)
        bad <- map_str_get<i32>(m, "missing")
        if bad == 0 { print("unreachable\n") }
    }
    => 0
}
"#,
        "map_str_get: missing key\n",
    );
}

#[test]
fn seeded_maps_match_rust_hashmaps() {
    use std::collections::HashMap;

    let mut rng = SeededRng(0x11a7_5eed_0a0b_1234);
    let int_keys: Vec<u64> = (0..12).collect();
    let str_keys: Vec<String> = vec![
        String::new(),
        "a".to_string(),
        "b".to_string(),
        "ab".to_string(),
        "ba".to_string(),
        "long-key".to_string(),
    ];

    let mut body = String::from("\nopen flow main\n    => i32\n{\n    protect {\n");
    body.push_str("        mut ints: IntMap<i32>\n        map_int_new<i32>(-> ints)\n");
    let mut expected = String::new();
    let mut int_ref: HashMap<u64, i32> = HashMap::new();
    for step in 0..48 {
        let key = int_keys[rng.next_u64() as usize % int_keys.len()];
        let value = rng.next_i32(-50, 50);
        body.push_str(&format!(
            "        map_int_set<i32>(<-> ints, {key}, {value})\n"
        ));
        int_ref.insert(key, value);
        if step == 17 || step == 41 {
            body.push_str(&format!(
                "        if map_int_len<i32>(ints) != {} {{ => {} }}\n",
                int_ref.len(),
                100 + step
            ));
        }
    }
    body.push_str("        printf(\"ilen:%llu\\n\", map_int_len<i32>(ints))\n");
    expected.push_str(&format!("ilen:{}\n", int_ref.len()));
    for (index, key) in int_keys.iter().enumerate() {
        let present = int_ref.contains_key(key);
        body.push_str(&format!(
            "        if map_int_contains<i32>(ints, {key}) {{ print(\"icontains{index}:1\\n\") }} else {{ print(\"icontains{index}:0\\n\") }}\n"
        ));
        expected.push_str(&format!("icontains{index}:{}\n", u8::from(present)));
        if let Some(value) = int_ref.get(key) {
            body.push_str(&format!(
                "        printf(\"ival{index}:%d\\n\", map_int_get<i32>(ints, {key}))\n"
            ));
            expected.push_str(&format!("ival{index}:{value}\n"));
        }
    }

    body.push_str("        mut words: StrMap<i32>\n        map_str_new<i32>(-> words)\n");
    let mut str_ref: HashMap<String, i32> = HashMap::new();
    for step in 0..48 {
        let key = &str_keys[rng.next_u64() as usize % str_keys.len()];
        let value = rng.next_i32(-50, 50);
        body.push_str(&format!(
            "        map_str_set<i32>(<-> words, \"{key}\", {value})\n"
        ));
        str_ref.insert(key.clone(), value);
        if step == 19 || step == 43 {
            body.push_str(&format!(
                "        if map_str_len<i32>(words) != {} {{ => {} }}\n",
                str_ref.len(),
                200 + step
            ));
        }
    }
    body.push_str("        printf(\"slen:%llu\\n\", map_str_len<i32>(words))\n");
    expected.push_str(&format!("slen:{}\n", str_ref.len()));
    for (index, key) in str_keys.iter().enumerate() {
        let present = str_ref.contains_key(key);
        body.push_str(&format!(
            "        if map_str_contains<i32>(words, \"{key}\") {{ print(\"scontains_{index}:1\\n\") }} else {{ print(\"scontains_{index}:0\\n\") }}\n"
        ));
        expected.push_str(&format!("scontains_{index}:{}\n", u8::from(present)));
        if let Some(value) = str_ref.get(key) {
            body.push_str(&format!(
                "        printf(\"svalue_{index}:%d\\n\", map_str_get<i32>(words, \"{key}\"))\n"
            ));
            expected.push_str(&format!("svalue_{index}:{value}\n"));
        }
    }

    body.push_str("        print(\"map properties ok\\n\")\n    }\n    => 0\n}\n");
    expected.push_str("map properties ok\n");

    assert_ok_with_c_variants("map-properties", &["map.loi"], &body, &[], &expected);
}

#[test]
fn map_imports_work_twice_and_across_libraries() {
    assert_ok_with_c_variants(
        "map-duplicate-imports",
        &["map.loi", "map.loi"],
        r#"
open flow main
    => i32
{
    protect {
        mut m: IntMap<i32>
        map_int_new<i32>(-> m)
        map_int_set<i32>(<-> m, 1, 2)
        mut s: StrMap<i32>
        map_str_new<i32>(-> s)
        map_str_set<i32>(<-> s, "one", 2)
        if map_int_get<i32>(m, 1) != map_str_get<i32>(s, "one") { => 1 }
        print("duplicate map imports ok\n")
    }
    => 0
}
"#,
        &[],
        "duplicate map imports ok\n",
    );

    let dir = test_dir("map-cross-library");
    let map_path = repo_root().join("lib").join("map.loi");
    let a = write_source(
        &dir,
        "wrapper_a.loi",
        &format!(
            "import \"{}\"\n\nopen flow int_digest\n    <- values: [i64]\n    => i64\n{{\n    protect {{\n        mut m: IntMap<i64>\n        map_int_new<i64>(-> m)\n        mut i: u64 <- 0\n        while i < values.len {{\n            map_int_set<i64>(<-> m, i, values[i])\n            i += 1\n        }}\n        mut total: i64 <- 0\n        i = 0\n        while i < values.len {{\n            total += map_int_get<i64>(m, i)\n            i += 1\n        }}\n        => total\n    }}\n}}\n",
            map_path.display()
        ),
    );
    let b = write_source(
        &dir,
        "wrapper_b.loi",
        &format!(
            "import \"{}\"\n\nopen flow str_digest\n    <- keys: [u8]\n    => i64\n{{\n    protect {{\n        mut m: StrMap<i64>\n        map_str_new<i64>(-> m)\n        map_str_set<i64>(<-> m, keys, 7)\n        => map_str_get<i64>(m, keys)\n    }}\n}}\n",
            map_path.display()
        ),
    );
    let program = write_source(
        &dir,
        "main.loi",
        &format!(
            "import \"{}\"\nimport \"{}\"\n\nopen flow main\n    => i32\n{{\n    protect {{\n        if int_digest([i64: 1, 2, 3]) != 6 or str_digest(\"key\") != 7 {{ => 1 }}\n        print(\"cross map imports ok\\n\")\n    }}\n    => 0\n}}\n",
            a.display(),
            b.display()
        ),
    );
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    if let Some((strict, ubsan)) = compile_c_variants(&dir, &program, &[]) {
        for executable in [strict, ubsan] {
            let run = Command::new(&executable)
                .output()
                .expect("C program should run");
            assert_eq!(
                run.status.code(),
                Some(0),
                "cross-library map import failed: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&run.stdout),
                "cross map imports ok\n"
            );
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn random_library_matches_known_vectors_and_shuffles_deterministically() {
    assert_ok_with_c_variants(
        "random-core",
        &["random.loi"],
        r#"
shape Pair [
    a: i32
    b: i32
]

open flow main
    => i32
{
    mut rng: Rng <- rng_new(1)
    if rng_state(rng) != 1 { => 1 }
    first <- rng_next_u64(<-> rng)
    second <- rng_next_u64(<-> rng)
    if first != 5180492295206395165 or second != 12380297144915551517 { => 2 }
    if rng_state(rng) != 1126174793148417 { => 3 }
    limited <- rng_below_u64(<-> rng, 10)
    narrow <- rng_next_u32(<-> rng)
    if limited != 3 or narrow != 1303648416 { => 4 }

    zero: Rng <- rng_new(0)
    if rng_state(zero) != 11400714819323198485 { => 5 }
    mut zero_draw: Rng <- rng_new(0)
    first_zero <- rng_next_u64(<-> zero_draw)
    if first_zero != 973819730272012410 { => 6 }

    mut other: Rng <- rng_new(831729)
    again <- rng_next_u64(<-> other)
    if again == 0 or rng_state(other) == 0 { => 7 }

    mut left: Rng <- rng_new(42)
    mut right: Rng <- rng_new(42)
    mut a: [i32] <- [i32: 1, 2, 3, 4, 5, 6, 7, 8]
    mut b: [i32] <- [i32: 1, 2, 3, 4, 5, 6, 7, 8]
    rng_shuffle<i32>(<-> left, <-> a)
    rng_shuffle<i32>(<-> right, <-> b)
    for i in 0..8 {
        if a[i] != b[i] { => 8 }
    }

    mut empty: [i32] <- [i32:]
    mut single: [i32] <- [i32: 9]
    rng_shuffle<i32>(<-> left, <-> empty)
    rng_shuffle<i32>(<-> left, <-> single)
    if empty.len != 0 or single[0] != 9 { => 9 }

    mut pairs: [Pair] <- [Pair[ a: 1, b: 2 ], Pair[ a: 3, b: 4 ], Pair[ a: 5, b: 6 ]]
    mut pair_rng: Rng <- rng_new(7)
    rng_shuffle<Pair>(<-> pair_rng, <-> pairs)
    if rng_pick<Pair>(<-> pair_rng, pairs).a < 1 { => 10 }

    print("random ok\n")
    => 0
}
"#,
        &[],
        "random ok\n",
    );
}

struct Xorshift(u64);

impl Xorshift {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(2685821657736338717)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }
}

#[test]
fn seeded_random_matches_rust_xorshift_reference() {
    let mut body = String::from("\nopen flow main\n    => i32\n{\n");
    let mut expected = String::new();

    for (seed_index, seed) in [1_u64, 0x0123_4567_89ab_cdef].into_iter().enumerate() {
        body.push_str(&format!(
            "    mut rng{seed_index}: Rng <- rng_new({seed})\n"
        ));
        let mut reference = Xorshift(seed);
        for step in 0..12 {
            match step % 3 {
                0 => {
                    let value = reference.next_u64();
                    body.push_str(&format!(
                        "    printf(\"n{seed_index}_{step}:%llu\\n\", rng_next_u64(<-> rng{seed_index}))\n"
                    ));
                    expected.push_str(&format!("n{seed_index}_{step}:{value}\n"));
                }
                1 => {
                    let value = reference.next_u64() >> 32;
                    body.push_str(&format!(
                        "    printf(\"u{seed_index}_{step}:%llu\\n\", rng_next_u32(<-> rng{seed_index}) as u64)\n"
                    ));
                    expected.push_str(&format!("u{seed_index}_{step}:{value}\n"));
                }
                _ => {
                    let bound = (step as u64) + 3;
                    let value = reference.below(bound);
                    body.push_str(&format!(
                        "    printf(\"b{seed_index}_{step}:%llu\\n\", rng_below_u64(<-> rng{seed_index}, {bound}))\n"
                    ));
                    expected.push_str(&format!("b{seed_index}_{step}:{value}\n"));
                }
            }
        }
    }

    let seed = 0xfeed_5eed_1234_5678;
    let values: Vec<i64> = (0..24).collect();
    body.push_str(&format!("    mut rng: Rng <- rng_new({seed})\n"));
    body.push_str("    mut shuffled: [i64] <- [i64: ");
    body.push_str(
        &values
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
    );
    body.push_str("]\n    rng_shuffle<i64>(<-> rng, <-> shuffled)\n");
    let mut reference = Xorshift(seed);
    let mut expected_values = values.clone();
    let mut i = expected_values.len() - 1;
    while i > 0 {
        let j = reference.below((i + 1) as u64) as usize;
        expected_values.swap(i, j);
        i -= 1;
    }
    for (index, value) in expected_values.iter().enumerate() {
        body.push_str(&format!(
            "    printf(\"s{index}:%lld\\n\", shuffled[{index}])\n"
        ));
        expected.push_str(&format!("s{index}:{value}\n"));
    }
    body.push_str("    print(\"random properties ok\\n\")\n    => 0\n}\n");
    expected.push_str("random properties ok\n");

    assert_ok_with_c_variants("random-properties", &["random.loi"], &body, &[], &expected);
}

#[test]
fn random_errors_name_the_flow() {
    assert_runtime_error(
        "random-bound-error",
        &["random.loi"],
        r#"
open flow main
    => i32
{
    mut rng: Rng <- rng_new(1)
    value <- rng_below_u64(<-> rng, 0)
    if value == 0 { print("unreachable\n") }
    => 0
}
"#,
        "rng_below_u64: bound is zero\n",
    );

    assert_runtime_error(
        "random-pick-error",
        &["random.loi"],
        r#"
open flow main
    => i32
{
    mut rng: Rng <- rng_new(1)
    empty: [i32] <- [i32:]
    value <- rng_pick<i32>(<-> rng, empty)
    if value == 0 { print("unreachable\n") }
    => 0
}
"#,
        "rng_pick: slice is empty\n",
    );
}

#[test]
fn random_imports_work_twice_and_across_libraries() {
    assert_ok_with_c_variants(
        "random-duplicate-imports",
        &["random.loi", "random.loi"],
        r#"
open flow main
    => i32
{
    mut rng: Rng <- rng_new(1)
    if rng_state(rng) != 1 or rng_next_u64(<-> rng) == 0 { => 1 }
    print("duplicate random imports ok\n")
    => 0
}
"#,
        &[],
        "duplicate random imports ok\n",
    );

    let dir = test_dir("random-cross-library");
    let random_path = repo_root().join("lib").join("random.loi");
    let a = write_source(
        &dir,
        "wrapper_a.loi",
        &format!(
            "import \"{}\"\n\nopen flow mixed\n    <- seed: u64\n    => u64\n{{\n    mut rng: Rng <- rng_new(seed)\n    first <- rng_next_u64(<-> rng)\n    second <- rng_next_u64(<-> rng)\n    => first ^ second\n}}\n",
            random_path.display()
        ),
    );
    let b = write_source(
        &dir,
        "wrapper_b.loi",
        &format!(
            "import \"{}\"\n\nopen flow shuffled_first\n    <- seed: u64\n    => i32\n{{\n    mut rng: Rng <- rng_new(seed)\n    mut xs: [i32] <- [i32: 1, 2, 3, 4]\n    rng_shuffle<i32>(<-> rng, <-> xs)\n    => xs[0]\n}}\n",
            random_path.display()
        ),
    );
    let program = write_source(
        &dir,
        "main.loi",
        &format!(
            "import \"{}\"\nimport \"{}\"\n\nopen flow main\n    => i32\n{{\n    mixed_value <- mixed(5)\n    first <- shuffled_first(5)\n    if mixed_value == 0 or first < 1 or first > 4 {{ => 1 }}\n    print(\"cross random imports ok\\n\")\n    => 0\n}}\n",
            a.display(),
            b.display()
        ),
    );
    let checked = check(&program);
    assert!(
        checked.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    if let Some((strict, ubsan)) = compile_c_variants(&dir, &program, &[]) {
        for executable in [strict, ubsan] {
            let run = Command::new(&executable)
                .output()
                .expect("C program should run");
            assert_eq!(
                run.status.code(),
                Some(0),
                "cross-library random import failed: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&run.stdout),
                "cross random imports ok\n"
            );
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn all_libraries_can_be_imported_together() {
    assert_ok(
        "all-libraries",
        &[
            "std.loi",
            "math.loi",
            "ctype.loi",
            "vec.loi",
            "slice.loi",
            "string.loi",
            "hash.loi",
            "crc32.loi",
            "map.loi",
            "random.loi",
            "os.loi",
            "net.loi",
        ],
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
    if hash_str("loi") == 0 or str_count("loi", 108) != 1 { => 7 }
    if crc32_bytes("loi") == 0 { => 8 }
    protect {
        mut m: IntMap<i32>
        map_int_new<i32>(-> m)
        map_int_set<i32>(<-> m, 1, 42)
        if map_int_get<i32>(m, 1) != 42 { => 9 }
        mut rng: Rng <- rng_new(1)
        if rng_next_u64(<-> rng) == 0 { => 10 }
    }
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

#[test]
fn public_library_apis_are_declared_in_generated_headers() {
    let dir = test_dir("public-header");
    let program = write_program(
        &dir,
        &[
            "vec.loi",
            "string.loi",
            "crc32.loi",
            "map.loi",
            "random.loi",
        ],
        r#"
open flow main
    => i32
{
    protect {
        mut xs: Vec<i32>
        vec_new<i32>(-> xs)
        vec_reserve<i32>(<-> xs, 16)
        seed <- vec_get<i32>(xs, 0)
        vec_push<i32>(<-> xs, seed)

        mut ints: IntMap<i32>
        map_int_new<i32>(-> ints)
        map_int_set<i32>(<-> ints, 1, seed)
        if map_int_get_or<i32>(ints, 1, -1) != seed { => 1 }
        mut found: bool
        mut value: i32
        map_int_lookup<i32>(ints, 1, -1, -> found, -> value)
        if not found or value != seed { => 2 }

        mut words: StrMap<i32>
        map_str_new<i32>(-> words)
        map_str_set<i32>(<-> words, "a", seed)
        if map_str_get_or<i32>(words, "a", -1) != seed { => 3 }
        map_str_lookup<i32>(words, "a", -1, -> found, -> value)
        if not found or value != seed { => 4 }

        trimmed <- str_trim(" a ")
        checksum <- crc32_extend(0, trimmed)
        mut rng: Rng <- rng_new(0)
        draw <- rng_next_u64(<-> rng)
        if checksum == draw as u32 { print("unreachable\n") }
    }
    => 0
}
"#,
    );
    let header = dir.join("public.h");
    let generated = Command::new(loi())
        .arg("gen")
        .arg(&program)
        .arg("-o")
        .arg(dir.join("public.c"))
        .arg("--emit-h")
        .arg(&header)
        .output()
        .expect("loi gen should run");
    assert!(
        generated.status.success(),
        "header generation failed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );

    let text = fs::read_to_string(&header).expect("header should be readable");
    for needle in [
        "typedef struct loi_t_Rng loi_t_Rng;",
        "typedef struct loi_t_Vec__i32 loi_t_Vec__i32;",
        "typedef struct loi_t_IntMap__i32 loi_t_IntMap__i32;",
        "typedef struct loi_t_StrMap__i32 loi_t_StrMap__i32;",
        "loi_fn_vec_new__i32(",
        "loi_fn_vec_reserve__i32(",
        "loi_fn_vec_get__i32(",
        "loi_fn_vec_push__i32(",
        "loi_fn_map_int_new__i32(",
        "loi_fn_map_int_set__i32(",
        "loi_fn_map_int_get_or__i32(",
        "loi_fn_map_int_lookup__i32(",
        "loi_fn_map_str_new__i32(",
        "loi_fn_map_str_set__i32(",
        "loi_fn_map_str_get_or__i32(",
        "loi_fn_map_str_lookup__i32(",
        "loi_fn_str_trim(",
        "loi_fn_crc32_extend(",
        "loi_fn_rng_new(",
    ] {
        assert!(text.contains(needle), "header missing {needle:?}");
    }
    let _ = fs::remove_dir_all(&dir);
}
