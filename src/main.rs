use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use std::collections::HashSet;

const USAGE: &str = "usage:\n  loi <lex|parse|fmt|check> <file.loi>\n  loi gen <file.loi> [-o output.c] [--emit-h header.h] [--source-map map.json]\n  loi build <file.loi> [-o output] [--emit exe|obj|lib]\n                    [--debug|--release] [--rebuild] [--emit-c file.c] [--emit-h header.h]\n                    [--arena-size BYTES] [--source-map map.json] [--cc-flag FLAG]...\n                    [--c-source FILE]...\n  loi clean <file.loi> [-o output]\n\n'gen', 'build' and 'clean' accept -o/--output.\n'--emit', '--debug', '--release', '--rebuild', '--emit-c', '--emit-h', '--arena-size', '--source-map', '--cc-flag' and '--c-source'\nare only supported by 'build'. '--c-source' compiles an extra C file (a vendored\nFFI shim) and links it into an executable or library; it is rejected with '--emit obj'.\n'--emit-h' and '--source-map' are also supported by 'gen'.\nWith '--emit obj' the default output is '<input stem>.o'; with '--emit lib'\nit is 'lib<input stem>.a'; otherwise it is './a.out'.\n'build' invokes 'cc' and 'ar'; set 'CC'/'AR' to override them. Those values\nmay contain arguments, split like a POSIX shell command line ('...' quotes,\n\"...\" quotes and backslash escapes).\nWithout -o, 'clean' removes ./a.out, <input stem>.o and lib<input stem>.a\nwhen their .loi-stamp files exist; with -o it removes exactly that output\nand its stamp. Both forms also remove generated C retained for that output\nthrough 'build --emit-c'. It does not read or check the source.\n\noptions:\n  -h, --help     show this help\n  -V, --version  show the compiler version\n  -g, --debug    compile with debug info (-g)\n      --release  compile with optimizations (-O2)\n      --rebuild  force recompilation, ignoring the incremental stamp\n      --arena-size BYTES  set the protect arena size (default 67108864 = 64 MiB)\n      --source-map FILE  write source map JSON Lines to FILE";

fn main() {
    let options = Options::parse();

    if options.mode == "clean" {
        clean_source(&options);
        return;
    }

    // Single-file modes read the source from the first path.
    let primary_path = options.paths.first().expect("path should exist");
    let source = match fs::read_to_string(primary_path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("loi: {}: {error}", primary_path.display());
            process::exit(1);
        }
    };

    match options.mode.as_str() {
        "lex" => lex(&source, primary_path),
        "parse" => parse_source(&source, primary_path),
        "fmt" => format_source(&source, primary_path),
        "check" => check_source(&source, primary_path),
        "gen" => gen_source(
            &source,
            primary_path,
            options.output.as_deref(),
            options.emit_h.as_deref(),
            options.source_map.as_deref(),
            options.emit_ir.as_deref(),
        ),
        "build" => build_source(
            &options.paths,
            primary_path,
            BuildRequest {
                output: options.output.as_deref(),
                kind: options.build_kind,
                profile: options.profile,
                rebuild: options.rebuild,
                emit_c: options.emit_c.as_deref(),
                emit_h: options.emit_h.as_deref(),
                arena_size: options.arena_size,
                source_map: options.source_map.as_deref(),
                emit_ir: options.emit_ir.as_deref(),
                cc_flags: &options.cc_flags,
                c_sources: &options.c_sources,
            },
        ),
        other => {
            eprintln!(
                "loi: unknown mode '{other}' (expected 'lex', 'parse', 'fmt', 'check', 'gen', 'build' or 'clean')"
            );
            process::exit(2);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BuildKind {
    Exe,
    Object,
    Library,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BuildProfile {
    Debug,
    Release,
}

impl BuildProfile {
    fn cc_flag(self) -> &'static str {
        match self {
            Self::Debug => "-g",
            Self::Release => "-O2",
        }
    }
}

impl BuildKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "exe" => Some(Self::Exe),
            "obj" | "object" => Some(Self::Object),
            "lib" | "library" => Some(Self::Library),
            _ => None,
        }
    }

    fn default_output(self, input: &Path) -> PathBuf {
        if self == Self::Exe {
            return PathBuf::from("a.out");
        }

        let stem = input
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("output");
        match self {
            Self::Exe => unreachable!(),
            Self::Object => PathBuf::from(format!("{stem}.o")),
            Self::Library => PathBuf::from(format!("lib{stem}.a")),
        }
    }
}

/// An external tool such as the C compiler, optionally preceded by extra
/// arguments taken from `CC`/`AR` (for example `ccache cc`).
struct Tool {
    program: OsString,
    args: Vec<OsString>,
}

impl Tool {
    fn from_env(variable: &str, default: &str) -> Self {
        match env::var(variable) {
            Ok(value) => match parse_tool(&value) {
                Some(tool) => tool,
                None => {
                    eprintln!("loi: environment variable {variable} is empty");
                    process::exit(2);
                }
            },
            Err(env::VarError::NotPresent) => Self {
                program: OsString::from(default),
                args: Vec::new(),
            },
            Err(env::VarError::NotUnicode(value)) => Self {
                program: value,
                args: Vec::new(),
            },
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        command
    }
}

/// Splits a tool value with POSIX-shell-like rules: whitespace separates
/// arguments, single quotes are literal, double quotes allow backslash
/// escapes, and a backslash escapes the next character outside quotes.
/// Returns `None` when the value contains no tokens.
fn parse_tool(value: &str) -> Option<Tool> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut chars = value.chars();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                started = true;
                for inner in chars.by_ref() {
                    if inner == '\'' {
                        break;
                    }
                    current.push(inner);
                }
            }
            '"' => {
                started = true;
                while let Some(inner) = chars.next() {
                    match inner {
                        '"' => break,
                        '\\' => {
                            if let Some(escaped) = chars.next() {
                                current.push(escaped);
                            }
                        }
                        _ => current.push(inner),
                    }
                }
            }
            '\\' => {
                started = true;
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            }
            ch if ch.is_whitespace() => {
                if started {
                    tokens.push(OsString::from(std::mem::take(&mut current)));
                    started = false;
                }
            }
            _ => {
                started = true;
                current.push(ch);
            }
        }
    }

    if started {
        tokens.push(OsString::from(current));
    }

    let mut tokens = tokens.into_iter();
    let program = tokens.next()?;
    if program.is_empty() {
        return None;
    }
    Some(Tool {
        program,
        args: tokens.collect(),
    })
}

struct Options {
    mode: String,
    paths: Vec<PathBuf>,
    output: Option<PathBuf>,
    build_kind: BuildKind,
    profile: Option<BuildProfile>,
    rebuild: bool,
    emit_c: Option<PathBuf>,
    emit_h: Option<PathBuf>,
    arena_size: Option<u64>,
    source_map: Option<PathBuf>,
    emit_ir: Option<PathBuf>,
    cc_flags: Vec<String>,
    c_sources: Vec<PathBuf>,
}

impl Options {
    fn parse() -> Self {
        let args = env::args().skip(1).collect::<Vec<_>>();

        match args.first().map(String::as_str) {
            Some("-h" | "--help") => {
                println!("{USAGE}");
                process::exit(0);
            }
            Some("-V" | "--version") => {
                println!("loi {}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            _ => {}
        }

        if args.len() < 2 {
            eprintln!("{USAGE}");
            process::exit(2);
        }

        let mode = args[0].clone();
        let mut paths = Vec::new();
        let mut output = None;
        let mut build_kind = None;
        let mut profile = None;
        let mut rebuild = false;
        let mut emit_c = None;
        let mut emit_h = None;
        let mut arena_size = None;
        let mut source_map = None;
        let mut emit_ir = None;
        let mut cc_flags = Vec::new();
        let mut c_sources = Vec::new();
        let mut index = 1;

        while index < args.len() {
            match args[index].as_str() {
                "-o" | "--output" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '{}' requires a path", args[index - 1]);
                        process::exit(2);
                    };
                    if output.replace(PathBuf::from(value)).is_some() {
                        eprintln!("loi: output path specified more than once");
                        process::exit(2);
                    }
                }
                "--emit" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--emit' requires 'exe', 'obj' or 'lib'");
                        process::exit(2);
                    };
                    let Some(kind) = BuildKind::parse(value) else {
                        eprintln!(
                            "loi: unknown emit kind '{value}' (expected 'exe', 'obj' or 'lib')"
                        );
                        process::exit(2);
                    };
                    if build_kind.replace(kind).is_some() {
                        eprintln!("loi: emit kind specified more than once");
                        process::exit(2);
                    }
                }
                "-g" | "--debug" => {
                    if profile.replace(BuildProfile::Debug).is_some() {
                        eprintln!("loi: build profile specified more than once");
                        process::exit(2);
                    }
                }
                "--release" => {
                    if profile.replace(BuildProfile::Release).is_some() {
                        eprintln!("loi: build profile specified more than once");
                        process::exit(2);
                    }
                }
                "--rebuild" => rebuild = true,
                "--emit-c" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--emit-c' requires a path");
                        process::exit(2);
                    };
                    if emit_c.replace(PathBuf::from(value)).is_some() {
                        eprintln!("loi: generated C path specified more than once");
                        process::exit(2);
                    }
                }
                "--emit-h" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--emit-h' requires a path");
                        process::exit(2);
                    };
                    if emit_h.replace(PathBuf::from(value)).is_some() {
                        eprintln!("loi: header path specified more than once");
                        process::exit(2);
                    }
                }
                "--arena-size" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--arena-size' requires a value");
                        process::exit(2);
                    };
                    let parsed: u64 = match value.parse() {
                        Ok(v) => v,
                        Err(_) => {
                            eprintln!("loi: invalid arena size '{value}'");
                            process::exit(2);
                        }
                    };
                    if arena_size.replace(parsed).is_some() {
                        eprintln!("loi: arena size specified more than once");
                        process::exit(2);
                    }
                }
                "--source-map" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--source-map' requires a path");
                        process::exit(2);
                    };
                    if source_map.replace(PathBuf::from(value)).is_some() {
                        eprintln!("loi: source map path specified more than once");
                        process::exit(2);
                    }
                }
                "--emit-ir" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--emit-ir' requires a path");
                        process::exit(2);
                    };
                    if emit_ir.replace(PathBuf::from(value)).is_some() {
                        eprintln!("loi: IR output path specified more than once");
                        process::exit(2);
                    }
                }
                "--emit-asm" => {
                    eprintln!(
                        "loi: '--emit-asm' is disabled: the x86-64 assembly backend is an unfinished draft and does not generate runnable code; use 'loi gen' or '--emit-c' and compile the result with cc"
                    );
                    process::exit(2);
                }
                "--cc-flag" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--cc-flag' requires a value");
                        process::exit(2);
                    };
                    cc_flags.push(value.clone());
                }
                "--c-source" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--c-source' requires a path");
                        process::exit(2);
                    };
                    c_sources.push(PathBuf::from(value));
                }
                other if other.starts_with('-') => {
                    eprintln!("loi: unknown option '{other}'");
                    process::exit(2);
                }
                other => {
                    paths.push(PathBuf::from(other));
                }
            }
            index += 1;
        }

        if paths.is_empty() {
            eprintln!("loi: no input file specified");
            process::exit(2);
        }

        // Single-file modes only use the first path.
        if !matches!(mode.as_str(), "build") && paths.len() > 1 {
            eprintln!("loi: '{}' accepts only one input file", mode);
            process::exit(2);
        }

        if output.is_some() && !matches!(mode.as_str(), "gen" | "build" | "clean") {
            eprintln!("loi: '-o' is only supported by 'gen', 'build' and 'clean'");
            process::exit(2);
        }
        if build_kind.is_some() && mode != "build" {
            eprintln!("loi: '--emit' is only supported by 'build'");
            process::exit(2);
        }
        if profile.is_some() && mode != "build" {
            eprintln!("loi: '--debug' and '--release' are only supported by 'build'");
            process::exit(2);
        }
        if rebuild && mode != "build" {
            eprintln!("loi: '--rebuild' is only supported by 'build'");
            process::exit(2);
        }
        if emit_c.is_some() && mode != "build" {
            eprintln!("loi: '--emit-c' is only supported by 'build'");
            process::exit(2);
        }
        if emit_h.is_some() && !matches!(mode.as_str(), "gen" | "build") {
            eprintln!("loi: '--emit-h' is only supported by 'gen' and 'build'");
            process::exit(2);
        }
        if arena_size.is_some() && mode != "build" {
            eprintln!("loi: '--arena-size' is only supported by 'build'");
            process::exit(2);
        }
        if emit_ir.is_some() && !matches!(mode.as_str(), "gen" | "build") {
            eprintln!("loi: '--emit-ir' is only supported by 'gen' and 'build'");
            process::exit(2);
        }
        if source_map.is_some() && !matches!(mode.as_str(), "gen" | "build") {
            eprintln!("loi: '--source-map' is only supported by 'gen' and 'build'");
            process::exit(2);
        }
        if !cc_flags.is_empty() && mode != "build" {
            eprintln!("loi: '--cc-flag' is only supported by 'build'");
            process::exit(2);
        }
        if !c_sources.is_empty() && mode != "build" {
            eprintln!("loi: '--c-source' is only supported by 'build'");
            process::exit(2);
        }

        // Multi-file build requires an output path (can't default to ./a.out with
        // multiple inputs without an explicit name).
        if mode == "build" && output.is_none() && paths.len() > 1 {
            eprintln!("loi: multi-file build requires '-o OUTPUT'");
            process::exit(2);
        }

        Self {
            mode,
            paths,
            output,
            build_kind: build_kind.unwrap_or(BuildKind::Exe),
            profile,
            rebuild,
            emit_c,
            emit_h,
            arena_size,
            source_map,
            emit_ir,
            cc_flags,
            c_sources,
        }
    }
}

fn lex(source: &str, path: &Path) {
    match loi::tokenize(source, &path.display().to_string()) {
        Ok(tokens) => {
            for token in &tokens {
                if token.kind == loi::TokenKind::Newline {
                    println!("{}:{} newline", token.line, token.column);
                } else {
                    println!(
                        "{}:{} {} '{}'",
                        token.line,
                        token.column,
                        token.kind.name(),
                        token.lexeme
                    );
                }
            }
        }
        Err(error) => {
            eprintln!("loi: {error}");
            process::exit(1);
        }
    }
}

fn parse_source(source: &str, path: &Path) {
    let program = load_program(source, path);
    println!("{program:#?}");
}

fn format_source(source: &str, path: &Path) {
    let program = load_program(source, path);
    print!("{}", loi::format_program(&program));
}

fn check_source(source: &str, path: &Path) {
    let program = load_program_with_imports(source, path);
    report_diagnostics(path, &loi::check(&program));
}

fn gen_source(
    source: &str,
    path: &Path,
    output: Option<&Path>,
    emit_h: Option<&Path>,
    source_map: Option<&Path>,
    emit_ir: Option<&Path>,
) {
    let program = load_program_with_imports(source, path);
    let c_source = generate_checked_program(&program, path, source_map);
    match output {
        Some(output) => {
            ensure_distinct_output(path, output);
            if let Err(error) = fs::write(output, c_source) {
                eprintln!("loi: {}: {error}", output.display());
                process::exit(1);
            }
        }
        None => print!("{c_source}"),
    }
    if let Some(header_path) = emit_h {
        ensure_distinct_output(path, header_path);
        match loi::generate_header(&program, Some(header_path)) {
            Ok(header) => {
                if let Err(error) = fs::write(header_path, header) {
                    eprintln!("loi: {}: {error}", header_path.display());
                    process::exit(1);
                }
            }
            Err(error) => {
                eprintln!("loi: {}: {error}", path.display());
                process::exit(1);
            }
        }
    }
    if let Some(ir_path) = emit_ir {
        match loi::generate_ir(&program) {
            Ok(ir) => {
                if let Err(error) = fs::write(ir_path, ir) {
                    eprintln!("loi: {}: {error}", ir_path.display());
                    process::exit(1);
                }
            }
            Err(error) => {
                eprintln!("loi: {}: {error}", path.display());
                process::exit(1);
            }
        }
    }
}

struct BuildRequest<'a> {
    output: Option<&'a Path>,
    kind: BuildKind,
    profile: Option<BuildProfile>,
    rebuild: bool,
    emit_c: Option<&'a Path>,
    emit_h: Option<&'a Path>,
    arena_size: Option<u64>,
    source_map: Option<&'a Path>,
    emit_ir: Option<&'a Path>,
    cc_flags: &'a [String],
    c_sources: &'a [PathBuf],
}

fn clean_source(options: &Options) {
    let primary_path = options.paths.first().expect("path should exist");

    // Single-file: use stamp-based cleaning (only remove default artifacts
    // that have a stamp, plus any explicit -o output).
    if options.paths.len() == 1 {
        let targets = match options.output.as_deref() {
            Some(output) => {
                ensure_distinct_output(primary_path, output);
                vec![output.to_path_buf()]
            }
            None => [BuildKind::Exe, BuildKind::Object, BuildKind::Library]
                .into_iter()
                .map(|kind| kind.default_output(primary_path))
                .filter(|output| build_stamp_path(output).is_file())
                .collect(),
        };

        for output in targets {
            let stamp = build_stamp_path(&output);
            if let Some(retained_c) = stamp_retained_c(&stamp)
                && retained_c.as_path() != primary_path
                && retained_c != output
            {
                remove_build_artifact(&retained_c);
            }
            remove_build_artifact(&output);
            remove_build_artifact(&stamp);
        }
        return;
    }

    // Multi-file: remove object files next to each source, plus the explicit
    // -o output if given.
    let mut targets = Vec::new();
    if let Some(output) = options.output.as_deref() {
        ensure_distinct_output(primary_path, output);
        targets.push(output.to_path_buf());
    }
    for input_path in &options.paths {
        let object_path = input_path.with_extension("o");
        if object_path.is_file() {
            targets.push(object_path);
        }
    }

    for target in targets {
        remove_build_artifact(&target);
    }
}

fn remove_build_artifact(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            eprintln!("loi: {}: {error}", path.display());
            process::exit(1);
        }
    }
}

fn build_source(paths: &[PathBuf], primary_path: &Path, request: BuildRequest<'_>) {
    let BuildRequest {
        output,
        kind,
        profile,
        rebuild,
        emit_c,
        emit_h,
        arena_size,
        source_map,
        emit_ir,
        cc_flags,
        c_sources,
    } = request;

    if !c_sources.is_empty() && kind == BuildKind::Object {
        eprintln!("loi: '--c-source' cannot be combined with '--emit obj'");
        process::exit(2);
    }

    let compiler = Tool::from_env("CC", "cc");
    let archiver = (kind == BuildKind::Library).then(|| Tool::from_env("AR", "ar"));
    let profile_flag = profile.map(|p| p.cc_flag());
    let arena_flag = arena_size.map(|size| format!("-DLOI_ARENA_SIZE={size}"));

    // Single-file build with retain/rebuild support uses the stamp-based
    // incremental logic. Multi-file builds always recompile all inputs.
    if paths.len() == 1 {
        let input_path = &paths[0];
        let source = match fs::read_to_string(input_path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("loi: {}: {error}", input_path.display());
                process::exit(1);
            }
        };

        let c_source = generate_checked(&source, input_path, source_map);
        let output_path = output
            .map(Path::to_path_buf)
            .unwrap_or_else(|| kind.default_output(input_path));
        ensure_distinct_output(input_path, &output_path);
        if let Some(emit_c) = emit_c {
            ensure_distinct_output(input_path, emit_c);
            if emit_c == output_path {
                eprintln!(
                    "loi: generated C path and build output must differ ('{}')",
                    output_path.display()
                );
                process::exit(2);
            }
        }
        if let Some(emit_h) = emit_h {
            ensure_distinct_output(input_path, emit_h);
            if emit_h == output_path {
                eprintln!(
                    "loi: header path and build output must differ ('{}')",
                    output_path.display()
                );
                process::exit(2);
            }
        }

        let fingerprint = build_fingerprint(
            &c_source,
            &compiler,
            archiver.as_ref(),
            kind,
            profile,
            cc_flags,
            c_sources,
            arena_size,
        );
        let stamp_path = build_stamp_path(&output_path);
        let retained_c_exists = match emit_c {
            Some(emit_c) => emit_c.is_file(),
            None => true,
        };
        let up_to_date = !rebuild
            && output_path.is_file()
            && retained_c_exists
            && stamp_matches(&stamp_path, &fingerprint, emit_c);

        let (c_path, temporary) = match emit_c {
            Some(emit_c) => (emit_c.to_path_buf(), false),
            None => (temporary_c_path(), true),
        };

        if (!temporary || !up_to_date)
            && let Err(error) = fs::write(&c_path, &c_source)
        {
            eprintln!("loi: {}: {error}", c_path.display());
            process::exit(1);
        }

        // Emit the header file if requested.
        if let Some(header_path) = emit_h {
            match loi::generate_header(&load_program(&source, input_path), Some(header_path)) {
                Ok(header) => {
                    if let Err(error) = fs::write(header_path, &header) {
                        eprintln!("loi: {}: {error}", header_path.display());
                        process::exit(1);
                    }
                }
                Err(error) => {
                    eprintln!("loi: {}: {error}", input_path.display());
                    process::exit(1);
                }
            }
        }

        // Emit IR if requested.
        if let Some(ir_path) = emit_ir {
            match loi::generate_ir(&load_program(&source, input_path)) {
                Ok(ir) => {
                    if let Err(error) = fs::write(ir_path, ir) {
                        eprintln!("loi: {}: {error}", ir_path.display());
                        process::exit(1);
                    }
                }
                Err(error) => {
                    eprintln!("loi: {}: {error}", input_path.display());
                    process::exit(1);
                }
            }
        }

        if up_to_date {
            return;
        }

        let success = match kind {
            BuildKind::Exe => compile_c(
                &compiler,
                profile,
                cc_flags,
                arena_flag.as_deref(),
                &output_path,
                &c_path,
                c_sources,
                false,
            ),
            BuildKind::Object => compile_c(
                &compiler,
                profile,
                cc_flags,
                arena_flag.as_deref(),
                &output_path,
                &c_path,
                &[],
                true,
            ),
            BuildKind::Library => build_library(
                &compiler,
                archiver
                    .as_ref()
                    .expect("archiver is configured for libraries"),
                profile,
                cc_flags,
                arena_flag.as_deref(),
                &output_path,
                &c_path,
                c_sources,
            ),
        };
        if temporary {
            let _ = fs::remove_file(&c_path);
        }

        if !success {
            process::exit(1);
        }

        let _ = fs::write(&stamp_path, stamp_contents(&fingerprint, emit_c));
        return;
    }

    // ------------------------------------------------------------------
    // Multi-file build: check the merged program once, then compile one
    // translation unit per input (with imported items folded into the first
    // unit that pulls them in) and link the objects.
    // ------------------------------------------------------------------

    let mut merged_items = Vec::new();
    let mut visited = HashSet::new();
    let mut units: Vec<(PathBuf, HashSet<String>, String)> = Vec::new();
    for input_path in paths {
        if !visited.insert(canonical_key(input_path)) {
            continue;
        }
        let source = match fs::read_to_string(input_path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("loi: {}: {error}", input_path.display());
                process::exit(1);
            }
        };
        let mut defined = HashSet::new();
        expand_program(
            &source,
            input_path,
            &mut visited,
            &mut merged_items,
            &mut defined,
        );
        units.push((input_path.clone(), defined, source));
    }
    let merged = loi::Program {
        items: merged_items,
    };
    report_diagnostics(primary_path, &loi::check(&merged));

    let mut object_files = Vec::new();
    for (input_path, defined, source) in &units {
        let c_source = match loi::generate_c_subset(&merged, source_map, defined) {
            Ok(c_source) => c_source,
            Err(error) => {
                eprintln!("loi: {}: {error}", input_path.display());
                process::exit(1);
            }
        };

        // Multi-file: use <stem>.o next to each source.
        let object_path = input_path.with_extension("o");
        ensure_distinct_output(input_path, &object_path);

        let c_path = temporary_c_path();
        if let Err(error) = fs::write(&c_path, &c_source) {
            eprintln!("loi: {}: {error}", c_path.display());
            process::exit(1);
        }

        // Emit header next to each source.
        let header_path = input_path.with_extension("h");
        ensure_distinct_output(input_path, &header_path);
        match loi::generate_header(
            &load_program(source.as_str(), input_path.as_path()),
            Some(header_path.as_path()),
        ) {
            Ok(header) => {
                if let Err(error) = fs::write(&header_path, &header) {
                    eprintln!("loi: {}: {error}", header_path.display());
                    process::exit(1);
                }
            }
            Err(error) => {
                eprintln!("loi: {}: {error}", input_path.display());
                process::exit(1);
            }
        }

        // Compile the C to an object file.
        let mut command = compiler.command();
        command.arg("-std=c11");
        if let Some(flag) = &profile_flag {
            command.arg(flag);
        }
        if let Some(flag) = &arena_flag {
            command.arg(flag);
        }
        for flag in cc_flags {
            command.arg(flag);
        }
        command.arg("-c").arg("-o").arg(&object_path).arg(&c_path);
        if !run_tool(&compiler, "C compiler", &mut command) {
            process::exit(1);
        }
        let _ = fs::remove_file(&c_path);
        object_files.push(object_path);
    }

    if kind == BuildKind::Library && !c_sources.is_empty() {
        eprintln!("loi: '--c-source' is not supported by multi-file library builds");
        process::exit(2);
    }

    match kind {
        BuildKind::Exe => {
            let output_path = output
                .map(Path::to_path_buf)
                .expect("multi-file exe requires -o");
            ensure_distinct_output(primary_path, &output_path);

            let mut command = compiler.command();
            command.arg("-std=c11");
            if let Some(flag) = &profile_flag {
                command.arg(flag);
            }
            if let Some(flag) = &arena_flag {
                command.arg(flag);
            }
            command.arg("-o").arg(&output_path);
            for obj in &object_files {
                command.arg(obj);
            }
            for source in c_sources {
                command.arg(source);
            }
            // Linker inputs (notably `-l` flags) must follow the objects.
            for flag in cc_flags {
                command.arg(flag);
            }
            command.arg("-lm");
            if !run_tool(&compiler, "C compiler/linker", &mut command) {
                process::exit(1);
            }
        }
        BuildKind::Object => {
            // Object files already written next to each source.
        }
        BuildKind::Library => {
            let output_path = output
                .map(Path::to_path_buf)
                .expect("multi-file lib requires -o");
            ensure_distinct_output(primary_path, &output_path);
            let Some(archiver) = archiver.as_ref() else {
                eprintln!("loi: archiver not configured");
                process::exit(1);
            };
            let archived = {
                let mut command = archiver.command();
                command.arg("rcs").arg(&output_path);
                for obj in &object_files {
                    command.arg(obj);
                }
                run_tool(archiver, "archiver", &mut command)
            };
            if !archived {
                process::exit(1);
            }
        }
    }
}

fn run_tool(tool: &Tool, description: &str, command: &mut Command) -> bool {
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(_) => false,
        Err(error) => {
            eprintln!(
                "loi: failed to run {description} '{}': {error}",
                tool.program.to_string_lossy()
            );
            false
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_fingerprint(
    c_source: &str,
    compiler: &Tool,
    archiver: Option<&Tool>,
    kind: BuildKind,
    profile: Option<BuildProfile>,
    cc_flags: &[String],
    c_sources: &[PathBuf],
    arena_size: Option<u64>,
) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    mix(&mut hash, env!("CARGO_PKG_VERSION").as_bytes());
    mix_tool(&mut hash, compiler);
    mix(
        &mut hash,
        &[match kind {
            BuildKind::Exe => 0,
            BuildKind::Object => 1,
            BuildKind::Library => 2,
        }],
    );
    mix(
        &mut hash,
        &[match profile {
            None => 0,
            Some(BuildProfile::Debug) => 1,
            Some(BuildProfile::Release) => 2,
        }],
    );
    for flag in cc_flags {
        mix(&mut hash, flag.as_bytes());
    }
    for source in c_sources {
        mix(&mut hash, source.to_string_lossy().as_bytes());
        if let Ok(contents) = fs::read(source) {
            mix(&mut hash, &contents);
        }
    }
    if let Some(size) = arena_size {
        mix(&mut hash, &size.to_le_bytes());
    }
    if let Some(archiver) = archiver {
        mix_tool(&mut hash, archiver);
    }
    mix(&mut hash, c_source.as_bytes());
    format!("{hash:016x}")
}

fn mix_tool(hash: &mut u64, tool: &Tool) {
    mix(hash, tool.program.to_string_lossy().as_bytes());
    mix(hash, &(tool.args.len() as u64).to_le_bytes());
    for arg in &tool.args {
        mix(hash, arg.to_string_lossy().as_bytes());
    }
}

fn mix(hash: &mut u64, bytes: &[u8]) {
    *hash ^= bytes.len() as u64;
    *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn build_stamp_path(output: &Path) -> PathBuf {
    let mut path = output.as_os_str().to_os_string();
    path.push(".loi-stamp");
    PathBuf::from(path)
}

fn stamp_matches(path: &Path, fingerprint: &str, retained_c: Option<&Path>) -> bool {
    let Ok(stamp) = fs::read_to_string(path) else {
        return false;
    };
    let mut lines = stamp.lines();
    if lines.next().map(str::trim) != Some(fingerprint) {
        return false;
    }
    let recorded = lines
        .find_map(|line| line.strip_prefix("emit-c="))
        .and_then(decode_stamp_path);
    recorded.as_deref() == retained_c
}

fn stamp_contents(fingerprint: &str, retained_c: Option<&Path>) -> String {
    let mut stamp = format!("{fingerprint}\n");
    if let Some(retained_c) = retained_c {
        stamp.push_str("emit-c=");
        stamp.push_str(&encode_stamp_path(retained_c));
        stamp.push('\n');
    }
    stamp
}

fn stamp_retained_c(path: &Path) -> Option<PathBuf> {
    let stamp = fs::read_to_string(path).ok()?;
    stamp
        .lines()
        .find_map(|line| line.strip_prefix("emit-c="))
        .and_then(decode_stamp_path)
}

/// Percent-encodes the characters that would break the line-oriented stamp
/// Percent-encodes the characters that would break the line-oriented stamp
/// format. Other characters (including non-ASCII UTF-8) are kept verbatim.
fn encode_stamp_path(path: &Path) -> String {
    let mut encoded = String::new();
    for ch in path.to_string_lossy().chars() {
        match ch {
            '%' => encoded.push_str("%25"),
            '\n' => encoded.push_str("%0A"),
            '\r' => encoded.push_str("%0D"),
            _ => encoded.push(ch),
        }
    }
    encoded
}

fn decode_stamp_path(text: &str) -> Option<PathBuf> {
    let mut decoded = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            decoded.push(ch);
            continue;
        }
        let hi = chars.next()?;
        let lo = chars.next()?;
        let byte = u8::from_str_radix(&format!("{hi}{lo}"), 16).ok()?;
        decoded.push(match byte {
            b'%' => '%',
            0x0A => '\n',
            0x0D => '\r',
            _ => return None,
        });
    }
    Some(PathBuf::from(decoded))
}

#[allow(clippy::too_many_arguments)]
fn compile_c(
    compiler: &Tool,
    profile: Option<BuildProfile>,
    cc_flags: &[String],
    arena_flag: Option<&str>,
    output: &Path,
    c_path: &Path,
    c_sources: &[PathBuf],
    object_only: bool,
) -> bool {
    if object_only && !c_sources.is_empty() {
        eprintln!("loi: '--c-source' cannot be combined with '--emit obj'");
        return false;
    }
    let mut command = compiler.command();
    command.arg("-std=c11");
    if let Some(profile) = profile {
        command.arg(profile.cc_flag());
    }
    if let Some(flag) = arena_flag {
        command.arg(flag);
    }
    if object_only {
        for flag in cc_flags {
            command.arg(flag);
        }
        command.arg("-c");
    }
    command.arg("-o").arg(output).arg(c_path);
    if !object_only {
        for source in c_sources {
            command.arg(source);
        }
        // Linker inputs (notably `-l` flags) must follow the objects that
        // reference them, so forward extra compiler flags here.
        for flag in cc_flags {
            command.arg(flag);
        }
        // Link with the math library so intrinsics such as `sqrt` resolve.
        command.arg("-lm");
    }
    run_tool(compiler, "C compiler", &mut command)
}

#[allow(clippy::too_many_arguments)]
fn build_library(
    compiler: &Tool,
    archiver: &Tool,
    profile: Option<BuildProfile>,
    cc_flags: &[String],
    arena_flag: Option<&str>,
    output: &Path,
    c_path: &Path,
    c_sources: &[PathBuf],
) -> bool {
    let object = temporary_object_path();
    let compiled = compile_c(
        compiler,
        profile,
        cc_flags,
        arena_flag,
        &object,
        c_path,
        &[],
        true,
    );
    let mut extra_objects = Vec::new();
    let mut extras_ok = true;
    if compiled {
        for source in c_sources {
            let extra = temporary_object_path();
            let ok = compile_c(
                compiler,
                profile,
                cc_flags,
                arena_flag,
                &extra,
                source,
                &[],
                true,
            );
            if ok {
                extra_objects.push(extra);
            } else {
                extras_ok = false;
                break;
            }
        }
    }
    let archived = if compiled && extras_ok {
        let mut command = archiver.command();
        command
            .arg("rcs")
            .arg(output)
            .arg(&object)
            .args(&extra_objects);
        run_tool(archiver, "archiver", &mut command)
    } else {
        false
    };
    let _ = fs::remove_file(&object);
    for extra in extra_objects {
        let _ = fs::remove_file(extra);
    }
    compiled && extras_ok && archived
}

fn temporary_object_path() -> PathBuf {
    temporary_path("o")
}

fn load_program(source: &str, path: &Path) -> loi::Program {
    match parse_source_program(source, path) {
        Ok(program) => program,
        Err(error) => {
            eprintln!("loi: {}:{error}", path.display());
            process::exit(1);
        }
    }
}

fn parse_source_program(source: &str, path: &Path) -> Result<loi::Program, String> {
    let tokens = loi::tokenize(source, &path.display().to_string()).map_err(|e| e.to_string())?;
    loi::parse(tokens).map_err(|e| e.to_string())
}

fn canonical_key(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn resolve_import_path(import: &str, from: &Path) -> PathBuf {
    let requested = Path::new(import);
    if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        from.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(requested)
    }
}

/// Expand `import` declarations in `source`, appending every non-import item to
/// `items`. Imported files are visited at most once per compilation unit.
fn expand_program(
    source: &str,
    path: &Path,
    visited: &mut HashSet<PathBuf>,
    items: &mut Vec<loi::Item>,
    names: &mut HashSet<String>,
) {
    let program = load_program(source, path);
    for item in program.items {
        if let loi::Item::Import(import) = &item {
            let child = resolve_import_path(&import.path, path);
            let key = canonical_key(&child);
            if !visited.insert(key) {
                continue;
            }
            let child_source = match fs::read_to_string(&child) {
                Ok(source) => source,
                Err(error) => {
                    eprintln!("loi: {}: {error}", child.display());
                    process::exit(1);
                }
            };
            expand_program(&child_source, &child, visited, items, names);
            continue;
        }
        if let Some(name) = item.declared_name() {
            names.insert(name.to_string());
        }
        items.push(item);
    }
}

/// Load a source file together with everything it imports.
fn load_program_with_imports(source: &str, path: &Path) -> loi::Program {
    let mut visited = HashSet::new();
    visited.insert(canonical_key(path));
    let mut items = Vec::new();
    let mut names = HashSet::new();
    expand_program(source, path, &mut visited, &mut items, &mut names);
    loi::Program { items }
}

fn generate_checked(source: &str, path: &Path, source_map_path: Option<&Path>) -> String {
    let program = load_program_with_imports(source, path);
    generate_checked_program(&program, path, source_map_path)
}

fn generate_checked_program(
    program: &loi::Program,
    path: &Path,
    source_map_path: Option<&Path>,
) -> String {
    report_diagnostics(path, &loi::check(program));

    match loi::generate_c(program, source_map_path) {
        Ok(c_source) => c_source,
        Err(error) => {
            eprintln!("loi: {}: {error}", path.display());
            process::exit(1);
        }
    }
}

fn ensure_distinct_output(input: &Path, output: &Path) {
    if input == output {
        eprintln!(
            "loi: output path must differ from input '{}'",
            input.display()
        );
        process::exit(2);
    }
}

fn report_diagnostics(path: &Path, diagnostics: &[loi::Diagnostic]) {
    if diagnostics.is_empty() {
        return;
    }

    for diagnostic in diagnostics {
        eprintln!(
            "{}:{}:{}: {}",
            path.display(),
            diagnostic.span.line,
            diagnostic.span.column,
            diagnostic.message
        );
    }
    process::exit(1);
}

fn temporary_c_path() -> PathBuf {
    temporary_path("c")
}

fn temporary_path(extension: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    env::temp_dir().join(format!("loi-build-{}-{nanos}.{extension}", process::id()))
}

#[cfg(test)]
mod tests {
    use super::{decode_stamp_path, encode_stamp_path, parse_tool};
    use std::path::Path;

    fn parts(value: &str) -> Option<(String, Vec<String>)> {
        parse_tool(value).map(|tool| {
            (
                tool.program.to_string_lossy().into_owned(),
                tool.args
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
            )
        })
    }

    #[test]
    fn tool_value_without_arguments() {
        assert_eq!(parts("cc"), Some(("cc".to_string(), vec![])));
    }

    #[test]
    fn tool_value_splits_whitespace_arguments() {
        assert_eq!(
            parts("ccache  cc\t-O2"),
            Some((
                "ccache".to_string(),
                vec!["cc".to_string(), "-O2".to_string()]
            ))
        );
    }

    #[test]
    fn tool_value_honours_quotes_and_escapes() {
        assert_eq!(
            parts(r#""/opt/my cc/bin/cc" -D'NAME=a b' -I/some\ path"#),
            Some((
                "/opt/my cc/bin/cc".to_string(),
                vec!["-DNAME=a b".to_string(), "-I/some path".to_string(),]
            ))
        );
        assert_eq!(
            parts(r#""cc" -DNAME="a b""#),
            Some(("cc".to_string(), vec!["-DNAME=a b".to_string()]))
        );
    }

    #[test]
    fn empty_tool_value_is_rejected() {
        assert_eq!(parts(""), None);
        assert_eq!(parts("   \t "), None);
        assert_eq!(parts("''"), None);
    }

    #[test]
    fn stamp_paths_round_trip_through_encoding() {
        for path in [
            "generated.c",
            "with space/generated.c",
            "percent%name.c",
            "line\nbreak.c",
            "carriage\rreturn.c",
            "unicode-ção.c",
        ] {
            let encoded = encode_stamp_path(Path::new(path));
            assert!(!encoded.contains('\n'), "{encoded}");
            assert!(!encoded.contains('\r'), "{encoded}");
            assert_eq!(
                decode_stamp_path(&encoded).as_deref(),
                Some(Path::new(path)),
                "round trip failed for {path:?}"
            );
        }
    }

    #[test]
    fn stamp_path_decoding_rejects_unknown_escapes() {
        assert_eq!(decode_stamp_path("%0B"), None);
        assert_eq!(decode_stamp_path("%2"), None);
    }
}
