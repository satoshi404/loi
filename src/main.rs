use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use std::collections::{HashMap, HashSet};

use loi::modules::{ModuleExport, ModuleView, ResolutionContext};
use loi::{ColorChoice, ErrorFormat};

const USAGE: &str = "usage:\n  loi <lex|parse|fmt> <file.loi>\n  loi check <file.loi> [--lib-dir DIR]...\n  loi complete <file.loi> --line N --column N\n  loi lsp-info <file.loi> --kind hover|definition|signature --line N --column N\n                          [--uri file:///path/to/file.loi]\n  loi gen <file.loi> [-o output.c] [--emit-h header.h] [--source-map map.json]\n                    [--emit-llvm-ir file.ll] [--lib-dir DIR]...\n  loi build <file.loi> [-o output] [--emit exe|obj|lib]\n                    [--debug|--release] [--rebuild] [--emit-c file.c] [--emit-h header.h]\n                    [--arena-size BYTES] [--source-map map.json] [--cc-flag FLAG]...\n                    [--c-source FILE]... [--auto-c-source] [--backend c|llvm]\n                    [--lib-dir DIR]... [--obj-dir DIR]\n  loi clean <file.loi> [-o output]\n  loi init NAME\n  loi make\n\n'gen', 'build' and 'clean' accept -o/--output.\n'init' creates a project with src/, build.loi, .gitignore and output/. 'make'\ncompiles and runs build.loi from the current project; it takes no file.\n'--emit', '--debug', '--release', '--rebuild', '--emit-c', '--emit-h', '--arena-size', '--source-map', '--cc-flag', '--c-source' and '--auto-c-source'\nare only supported by 'build'. '--c-source' compiles an extra C file (a vendored\nFFI shim) and links it into an executable or library; it is rejected with '--emit obj'.\n'--auto-c-source' links shim files discovered from imported packages; it is also\nrejected with '--emit obj' and by the LLVM backend.\n'--obj-dir DIR' writes intermediate objects and headers to DIR instead of beside\nthe sources; it is supported by 'build' and forces the multi-object pipeline.\n'--lib-dir DIR' adds a library root for logical imports, checked after any\nnearer project lib/ and in the order given; repeatable. It is supported by\n'check', 'gen' and 'build' and never uses the process cwd.\n'--emit-h', '--source-map' and '--emit-llvm-ir' are also supported by 'gen'.\n'--line' and '--column' are supported by 'complete' and 'lsp-info'; positions are\n1-based. 'complete' prints candidates, while 'lsp-info' prints a ready-to-use\nLSP hover, definition or signature-help result.\n'--backend' selects the LLVM opt-in backend (C11 is the default); it supports\nonly single-file executables and rejects C-only build options.\nWith '--emit obj' the default output is '<input stem>.o'; with '--emit lib'\nit is 'lib<input stem>.a'; otherwise it is './a.out'.\n'build' invokes 'cc' and 'ar'; set 'CC'/'AR' to override them. Those values\nmay contain arguments, split like a POSIX shell command line ('...' quotes,\n\"...\" quotes and backslash escapes).\nWithout -o, 'clean' removes ./a.out, <input stem>.o and lib<input stem>.a\nwhen their .loi-stamp files exist; with -o it removes exactly that output\nand its stamp. Both forms also remove generated C retained for that output\nthrough 'build --emit-c'. It does not read or check the source.\n\noptions:\n  -h, --help     show this help\n  -V, --version  show the compiler version\n  -g, --debug    compile with debug info (-g)\n      --release  compile with optimizations (-O2)\n      --rebuild  force recompilation, ignoring the incremental stamp\n      --arena-size BYTES  set the protect arena size (default 67108864 = 64 MiB)\n      --source-map FILE  write source map JSON Lines to FILE\n      --error-format FMT  diagnostic format: human, short or json (default human)\n      --color WHEN   colorize diagnostics: auto, always or never (default auto)\n      --line N      1-based line for 'complete' (LSP line + 1)\n      --column N    1-based column for 'complete' (LSP character + 1)";

fn main() {
    let options = Options::parse();

    if options.mode == "init" {
        init_project(&options.paths[0]);
        return;
    }
    if options.mode == "make" {
        make_project();
        return;
    }
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
    let diagnostics = DiagnosticOptions {
        format: options.error_format,
        color: options.color,
    };

    if options.mode == "build" && options.backend.as_deref() == Some("llvm") {
        build_llvm_source(&options, primary_path, &source, diagnostics);
        return;
    }

    match options.mode.as_str() {
        "lex" => lex(&source, primary_path, diagnostics),
        "parse" => parse_source(&source, primary_path, diagnostics),
        "fmt" => format_source(&source, primary_path, diagnostics),
        "check" => check_source(&source, primary_path, diagnostics, &options.lib_dirs),
        "complete" => complete_source(
            &source,
            primary_path,
            options.line.expect("'--line' is required for 'complete'"),
            options
                .column
                .expect("'--column' is required for 'complete'"),
        ),
        "lsp-info" => lsp_info_source(
            &source,
            primary_path,
            options.line.unwrap_or(1),
            options.column.unwrap_or(1),
            options.uri.as_deref().unwrap_or(""),
            options
                .kind
                .as_deref()
                .and_then(loi::InfoKind::parse)
                .expect("'--kind' is validated for 'lsp-info'"),
            options.new_name.as_deref().unwrap_or(""),
        ),
        "gen" => gen_source(
            &source,
            primary_path,
            options.output.as_deref(),
            options.emit_h.as_deref(),
            options.source_map.as_deref(),
            options.emit_ir.as_deref(),
            options.emit_llvm_ir.as_deref(),
            diagnostics,
            &options.lib_dirs,
        ),
        "build" => build_source(
            &options.paths,
            primary_path,
            BuildRequest {
                output: options.output.as_deref(),
                obj_dir: options.obj_dir.as_deref(),
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
                auto_c_source: options.auto_c_source,
                lib_dirs: &options.lib_dirs,
            },
            diagnostics,
        ),
        other => {
            eprintln!(
                "loi: unknown mode '{other}' (expected 'lex', 'parse', 'fmt', 'check', 'complete', 'lsp-info', 'gen', 'build', 'clean', 'init' or 'make')"
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
    obj_dir: Option<PathBuf>,
    build_kind: BuildKind,
    profile: Option<BuildProfile>,
    rebuild: bool,
    emit_c: Option<PathBuf>,
    emit_h: Option<PathBuf>,
    arena_size: Option<u64>,
    source_map: Option<PathBuf>,
    emit_ir: Option<PathBuf>,
    emit_llvm_ir: Option<PathBuf>,
    backend: Option<String>,
    cc_flags: Vec<String>,
    c_sources: Vec<PathBuf>,
    auto_c_source: bool,
    lib_dirs: Vec<PathBuf>,
    error_format: ErrorFormat,
    color: ColorChoice,
    line: Option<usize>,
    column: Option<usize>,
    kind: Option<String>,
    uri: Option<String>,
    new_name: Option<String>,
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

        if args.len() < 2 && args.first().map(String::as_str) != Some("make") {
            eprintln!("{USAGE}");
            process::exit(2);
        }

        let mode = args[0].clone();
        let mut paths = Vec::new();
        let mut output = None;
        let mut obj_dir = None;
        let mut build_kind = None;
        let mut profile = None;
        let mut rebuild = false;
        let mut emit_c = None;
        let mut emit_h = None;
        let mut arena_size = None;
        let mut source_map = None;
        let mut emit_ir = None;
        let mut emit_llvm_ir = None;
        let mut backend = None;
        let mut cc_flags = Vec::new();
        let mut c_sources = Vec::new();
        let mut auto_c_source = false;
        let mut lib_dirs = Vec::new();
        let mut error_format = ErrorFormat::Human;
        let mut color = ColorChoice::default();
        let mut line = None;
        let mut column = None;
        let mut kind = None;
        let mut uri = None;
        let mut new_name = None;
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
                "--obj-dir" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--obj-dir' requires a path");
                        process::exit(2);
                    };
                    if obj_dir.replace(PathBuf::from(value)).is_some() {
                        eprintln!("loi: object directory specified more than once");
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
                "--emit-llvm-ir" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--emit-llvm-ir' requires a path");
                        process::exit(2);
                    };
                    if emit_llvm_ir.replace(PathBuf::from(value)).is_some() {
                        eprintln!("loi: LLVM IR output path specified more than once");
                        process::exit(2);
                    }
                }
                "--backend" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--backend' requires 'c' or 'llvm'");
                        process::exit(2);
                    };
                    if value != "c" && value != "llvm" {
                        eprintln!("loi: unknown backend '{value}' (expected 'c' or 'llvm')");
                        process::exit(2);
                    }
                    if backend.replace(value.clone()).is_some() {
                        eprintln!("loi: backend specified more than once");
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
                "--auto-c-source" => {
                    if auto_c_source {
                        eprintln!("loi: '--auto-c-source' specified more than once");
                        process::exit(2);
                    }
                    auto_c_source = true;
                }
                "--lib-dir" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--lib-dir' requires a path");
                        process::exit(2);
                    };
                    let dir = PathBuf::from(value);
                    if !dir.is_dir() {
                        eprintln!("loi: '--lib-dir' is not a directory: {}", dir.display());
                        process::exit(2);
                    }
                    lib_dirs.push(dir);
                }
                "--line" | "--column" => {
                    let flag = args[index].clone();
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '{flag}' requires a positive integer");
                        process::exit(2);
                    };
                    let Ok(parsed) = value.parse::<usize>() else {
                        eprintln!("loi: '{flag}' requires a positive integer, got '{value}'");
                        process::exit(2);
                    };
                    if parsed == 0 {
                        eprintln!("loi: '{flag}' must be at least 1");
                        process::exit(2);
                    }
                    let target = if flag == "--line" {
                        &mut line
                    } else {
                        &mut column
                    };
                    if target.replace(parsed).is_some() {
                        eprintln!("loi: '{flag}' specified more than once");
                        process::exit(2);
                    }
                }
                "--kind" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--kind' requires an LSP info kind");
                        process::exit(2);
                    };
                    if loi::InfoKind::parse(value).is_none() {
                        eprintln!(
                            "loi: unknown info kind '{value}' (expected hover, definition, signature, references, prepareRename, rename, documentSymbol, semanticTokens, inlayHint or foldingRange)"
                        );
                        process::exit(2);
                    }
                    if kind.replace(value.clone()).is_some() {
                        eprintln!("loi: '--kind' specified more than once");
                        process::exit(2);
                    }
                }
                "--uri" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--uri' requires a document URI");
                        process::exit(2);
                    };
                    if uri.replace(value.clone()).is_some() {
                        eprintln!("loi: '--uri' specified more than once");
                        process::exit(2);
                    }
                }
                "--new-name" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--new-name' requires a name");
                        process::exit(2);
                    };
                    if new_name.replace(value.clone()).is_some() {
                        eprintln!("loi: '--new-name' specified more than once");
                        process::exit(2);
                    }
                }
                "--error-format" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--error-format' requires 'human', 'short' or 'json'");
                        process::exit(2);
                    };
                    let Some(parsed) = ErrorFormat::parse(value) else {
                        eprintln!(
                            "loi: unknown error format '{value}' (expected 'human', 'short' or 'json')"
                        );
                        process::exit(2);
                    };
                    error_format = parsed;
                }
                value if value.starts_with("--error-format=") => {
                    let value = &value["--error-format=".len()..];
                    let Some(parsed) = ErrorFormat::parse(value) else {
                        eprintln!(
                            "loi: unknown error format '{value}' (expected 'human', 'short' or 'json')"
                        );
                        process::exit(2);
                    };
                    error_format = parsed;
                }
                "--color" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        eprintln!("loi: '--color' requires 'auto', 'always' or 'never'");
                        process::exit(2);
                    };
                    let Some(parsed) = ColorChoice::parse(value) else {
                        eprintln!(
                            "loi: unknown color choice '{value}' (expected 'auto', 'always' or 'never')"
                        );
                        process::exit(2);
                    };
                    color = parsed;
                }
                value if value.starts_with("--color=") => {
                    let value = &value["--color=".len()..];
                    let Some(parsed) = ColorChoice::parse(value) else {
                        eprintln!(
                            "loi: unknown color choice '{value}' (expected 'auto', 'always' or 'never')"
                        );
                        process::exit(2);
                    };
                    color = parsed;
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

        if paths.is_empty() && mode != "make" {
            eprintln!("loi: no input file specified");
            process::exit(2);
        }
        if mode == "make" && !paths.is_empty() {
            eprintln!("loi: 'make' does not accept input files");
            process::exit(2);
        }
        if mode == "init" && paths.len() != 1 {
            eprintln!("loi: 'init' requires exactly one project name");
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
        if obj_dir.is_some() && mode != "build" {
            eprintln!("loi: '--obj-dir' is only supported by 'build'");
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
        if emit_llvm_ir.is_some() && mode != "gen" {
            eprintln!("loi: '--emit-llvm-ir' is only supported by 'gen'");
            process::exit(2);
        }
        if backend.is_some() && mode != "build" {
            eprintln!("loi: '--backend' is only supported by 'build'");
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
        if auto_c_source && mode != "build" {
            eprintln!("loi: '--auto-c-source' is only supported by 'build'");
            process::exit(2);
        }
        if !lib_dirs.is_empty() && !matches!(mode.as_str(), "check" | "gen" | "build") {
            eprintln!("loi: '--lib-dir' is only supported by 'check', 'gen' and 'build'");
            process::exit(2);
        }
        if (line.is_some() || column.is_some()) && !matches!(mode.as_str(), "complete" | "lsp-info")
        {
            eprintln!(
                "loi: '--line' and '--column' are only supported by 'complete' and 'lsp-info'"
            );
            process::exit(2);
        }
        if mode == "complete" && (line.is_none() || column.is_none()) {
            eprintln!("loi: 'complete' requires both '--line' and '--column'");
            process::exit(2);
        }
        if kind.is_some() && mode != "lsp-info" {
            eprintln!("loi: '--kind' is only supported by 'lsp-info'");
            process::exit(2);
        }
        if uri.is_some() && mode != "lsp-info" {
            eprintln!("loi: '--uri' is only supported by 'lsp-info'");
            process::exit(2);
        }
        if new_name.is_some() && mode != "lsp-info" {
            eprintln!("loi: '--new-name' is only supported by 'lsp-info'");
            process::exit(2);
        }
        if mode == "lsp-info" {
            let Some(kind_value) = kind.as_deref() else {
                eprintln!("loi: 'lsp-info' requires '--kind'");
                process::exit(2);
            };
            let parsed_kind = loi::InfoKind::parse(kind_value).unwrap_or(loi::InfoKind::Hover);
            let needs_position = !matches!(
                parsed_kind,
                loi::InfoKind::DocumentSymbol
                    | loi::InfoKind::SemanticTokens
                    | loi::InfoKind::FoldingRange
                    | loi::InfoKind::InlayHint
            );
            if needs_position && (line.is_none() || column.is_none()) {
                eprintln!(
                    "loi: 'lsp-info --kind {kind_value}' requires both '--line' and '--column'"
                );
                process::exit(2);
            }
            if matches!(parsed_kind, loi::InfoKind::Rename) && new_name.is_none() {
                eprintln!("loi: 'lsp-info --kind rename' requires '--new-name'");
                process::exit(2);
            }
        }

        // Multi-object builds (several inputs or an explicit object directory)
        // need an explicit output name instead of defaulting to ./a.out.
        if mode == "build" && output.is_none() && (paths.len() > 1 || obj_dir.is_some()) {
            eprintln!("loi: multi-file build requires '-o OUTPUT'");
            process::exit(2);
        }

        Self {
            mode,
            paths,
            output,
            obj_dir,
            build_kind: build_kind.unwrap_or(BuildKind::Exe),
            profile,
            rebuild,
            emit_c,
            emit_h,
            arena_size,
            source_map,
            emit_ir,
            emit_llvm_ir,
            backend,
            cc_flags,
            c_sources,
            auto_c_source,
            lib_dirs,
            error_format,
            color,
            line,
            column,
            kind,
            uri,
            new_name,
        }
    }
}

fn invalid_project_name(value: &Path) -> ! {
    eprintln!(
        "loi: invalid project name '{}' (use letters, digits, '-' or '_', starting with a letter or '_')",
        value.display()
    );
    process::exit(2);
}

fn write_generated(path: &Path, contents: &[u8]) {
    if let Err(error) = fs::write(path, contents) {
        eprintln!("loi: {}: {error}", path.display());
        process::exit(1);
    }
}

/// Create the deterministic project layout used by `loi make`.
fn init_project(path: &Path) {
    let Some(name) = path.to_str() else {
        invalid_project_name(path);
    };
    let valid = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if !valid {
        invalid_project_name(path);
    }

    let project = Path::new(name);
    match fs::symlink_metadata(project) {
        Ok(_) => {
            eprintln!("loi: project directory '{name}' already exists");
            process::exit(2);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            eprintln!("loi: {name}: {error}");
            process::exit(1);
        }
    }

    for dir in [project.join("src"), project.join("output/objects")] {
        if let Err(error) = fs::create_dir_all(&dir) {
            eprintln!("loi: {}: {error}", dir.display());
            process::exit(1);
        }
    }

    let main = format!(
        "// Generated by `loi init {name}`.\n\
         open flow main\n\
             => i32\n\
         {{\n\
             print(\"hello, {name}\\n\")\n\
             => 0\n\
         }}\n"
    );
    let build_script = format!(
        r#"// Generated by `loi init`.
// `loi make` compiles and runs this file. Add `lib/build.loi` locally to override.
import build as loi_build

open flow main
    => i32
{{
    protect {{
        mut plan: loi_build::BuildPlan
        loi_build::new("{name}", -> plan)
        loi_build::source(<-> plan, "src/main.loi")
        loi_build::mode(<-> plan, "native")
        loi_build::optimize(<-> plan, "none")
        => loi_build::run(plan)
    }}
}}
"#
    );
    let config = format!(
        "# Generated by `loi init {name}`. Informational snapshot: the build.loi plan is authoritative.\n\
         name={name}\n\
         target=exe\n\
         profile=debug\n\
         source=src/main.loi\n"
    );

    write_generated(&project.join("src/main.loi"), main.as_bytes());
    write_generated(&project.join("build.loi"), build_script.as_bytes());
    write_generated(&project.join(".gitignore"), b"/output/\n");
    write_generated(&project.join("output/config.txt"), config.as_bytes());
    write_generated(&project.join("output/objects/.gitkeep"), b"");

    println!("loi: created project '{name}'");
}

/// Locate the toolchain library root. An explicit `LOI_TOOLCHAIN_LIB` wins;
/// otherwise look beside the running compiler and finally at the compile-time
/// manifest directory, which keeps `cargo test` and local builds working.
fn toolchain_lib_dir() -> Option<PathBuf> {
    if let Some(value) = env::var_os("LOI_TOOLCHAIN_LIB") {
        let path = PathBuf::from(value);
        return path.is_dir().then_some(path);
    }
    let exe = env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        let candidate = ancestor.join("lib");
        if candidate.join("build.loi").is_file() {
            return Some(candidate);
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lib");
    manifest.join("build.loi").is_file().then_some(manifest)
}

/// Compile `build.loi` with this compiler, then execute it with the variables
/// the Loi runner needs. Build semantics live in `lib/build.loi`; this driver
/// only locates files, forwards environment variables and relays exit codes.
fn make_project() {
    let script = Path::new("build.loi");
    if !script.is_file() {
        eprintln!("loi: no build.loi in the current directory (run 'loi init NAME')");
        process::exit(2);
    }

    let Some(lib_dir) = toolchain_lib_dir() else {
        eprintln!(
            "loi: could not locate lib/build.loi; set LOI_TOOLCHAIN_LIB to the toolchain library directory"
        );
        process::exit(2);
    };
    let process_shim = lib_dir.join("process_shim.c");
    if !process_shim.is_file() {
        eprintln!("loi: missing toolchain shim '{}'", process_shim.display());
        process::exit(2);
    }
    let Ok(self_path) = env::current_exe() else {
        eprintln!("loi: could not locate the running compiler");
        process::exit(1);
    };
    for dir in [Path::new("output"), Path::new("output/objects")] {
        if let Err(error) = fs::create_dir_all(dir) {
            eprintln!("loi: {}: {error}", dir.display());
            process::exit(1);
        }
    }

    let runner_dir = env::temp_dir().join(format!("loi-make-{}", process::id()));
    let _ = fs::remove_dir_all(&runner_dir);
    if let Err(error) = fs::create_dir_all(&runner_dir) {
        eprintln!("loi: {}: {error}", runner_dir.display());
        process::exit(1);
    }
    let runner = runner_dir.join("build-runner");

    let compile_status = Command::new(&self_path)
        .arg("build")
        .arg(script)
        .arg("-o")
        .arg(&runner)
        .arg("--debug")
        .arg("--auto-c-source")
        .arg("--c-source")
        .arg(&process_shim)
        .arg("--lib-dir")
        .arg(&lib_dir)
        .status();
    let compiled = match compile_status {
        Ok(status) if status.success() => true,
        Ok(status) => {
            let _ = fs::remove_dir_all(&runner_dir);
            process::exit(status.code().unwrap_or(1));
        }
        Err(error) => {
            eprintln!("loi: failed to run '{}': {error}", self_path.display());
            let _ = fs::remove_dir_all(&runner_dir);
            process::exit(1);
        }
    };
    assert!(compiled);

    let run_status = Command::new(&runner)
        .env("LOI_SELF", &self_path)
        .env("LOI_TOOLCHAIN_LIB", &lib_dir)
        .status();
    let code = match run_status {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("loi: failed to run '{}': {error}", runner.display());
            1
        }
    };
    let _ = fs::remove_dir_all(&runner_dir);
    process::exit(code);
}

fn lex(source: &str, path: &Path, diagnostics: DiagnosticOptions) {
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
        Err(error) => report_one_diagnostic(
            path,
            source,
            loi::Diagnostic::from_error(
                error.message,
                frontend_span(error.line, error.column, Some(error.end_column)),
            ),
            diagnostics,
        ),
    }
}

fn parse_source(source: &str, path: &Path, diagnostics: DiagnosticOptions) {
    let program = load_program(source, path, diagnostics);
    println!("{program:#?}");
}

fn format_source(source: &str, path: &Path, diagnostics: DiagnosticOptions) {
    let filename = path.display().to_string();
    let (tokens, comments) = match loi::tokenize_with_comments(source, &filename) {
        Ok(parts) => parts,
        Err(error) => report_one_diagnostic(
            path,
            source,
            loi::Diagnostic::from_error(
                error.message,
                frontend_span(error.line, error.column, Some(error.end_column)),
            ),
            diagnostics,
        ),
    };
    let program = match loi::parse(tokens) {
        Ok(program) => program,
        Err(error) => report_one_diagnostic(
            path,
            source,
            loi::Diagnostic::from_error(error.message, loi::Span::new(error.line, error.column)),
            diagnostics,
        ),
    };
    print!("{}", loi::format_program_with_comments(&program, &comments));
}

fn check_source(source: &str, path: &Path, diagnostics: DiagnosticOptions, lib_dirs: &[PathBuf]) {
    let (program, primary) = load_program_with_imports(source, path, diagnostics, lib_dirs);
    report_diagnostics(
        path,
        source,
        &loi::check_with_warnings_from(&program, &primary),
        diagnostics,
    );
}

/// Print completion candidates for a 1-based cursor position as compact JSON.
/// Completion runs on the raw source, so an unfinished buffer still yields
/// candidates and never emits compiler diagnostics into the editor's log.
fn complete_source(source: &str, path: &Path, line: usize, column: usize) {
    let items = loi::completions(source, &path.display().to_string(), line, column);
    println!("{}", loi::completions_to_json(&items));
}

/// Print a ready-to-use LSP result for editor queries.
/// Like completion, this reads the raw buffer so an incomplete document still
/// answers editor requests without emitting compiler diagnostics.
fn lsp_info_source(
    source: &str,
    path: &Path,
    line: usize,
    column: usize,
    uri: &str,
    kind: loi::InfoKind,
    new_name: &str,
) {
    print!(
        "{}",
        loi::lsp_info_json(
            source,
            &path.display().to_string(),
            line,
            column,
            uri,
            kind,
            new_name
        )
    );
    let _ = std::io::stdout().flush();
}

#[allow(clippy::too_many_arguments)]
fn gen_source(
    source: &str,
    path: &Path,
    output: Option<&Path>,
    emit_h: Option<&Path>,
    source_map: Option<&Path>,
    emit_ir: Option<&Path>,
    emit_llvm_ir: Option<&Path>,
    diagnostics: DiagnosticOptions,
    lib_dirs: &[PathBuf],
) {
    let (program, primary) = load_program_with_imports(source, path, diagnostics, lib_dirs);
    let c_source =
        generate_checked_program(&program, &primary, path, source, source_map, diagnostics);
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
        match loi::generate_header_named(&program, None, Some(header_path)) {
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
    if let Some(ir_path) = emit_llvm_ir {
        ensure_distinct_output(path, ir_path);
        match loi::generate_llvm_ir(&program) {
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
    obj_dir: Option<&'a Path>,
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
    auto_c_source: bool,
    lib_dirs: &'a [PathBuf],
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

/// Compile the scalar subset through textual LLVM IR. This path is opt-in:
/// `build --backend llvm` stays separate from the C11 production pipeline and
/// deliberately shares neither its incremental stamp nor its C-only options.
fn build_llvm_source(
    options: &Options,
    input_path: &Path,
    source: &str,
    diagnostics: DiagnosticOptions,
) {
    let reject = |message: &str| -> ! {
        eprintln!("loi: LLVM backend {message}");
        process::exit(2);
    };

    if options.paths.len() != 1 {
        reject("supports only a single input file");
    }
    if options.build_kind != BuildKind::Exe {
        reject("supports only executable builds");
    }
    if options.emit_c.is_some() {
        reject("does not support '--emit-c'");
    }
    if options.emit_h.is_some() {
        reject("does not support '--emit-h'");
    }
    if options.emit_ir.is_some() {
        reject("does not support '--emit-ir'");
    }
    if options.emit_llvm_ir.is_some() {
        reject("does not support '--emit-llvm-ir'");
    }
    if options.arena_size.is_some() {
        reject("does not support '--arena-size'");
    }
    if options.source_map.is_some() {
        reject("does not support '--source-map'");
    }
    if !options.c_sources.is_empty() {
        reject("does not support '--c-source'");
    }
    if options.auto_c_source {
        reject("does not support '--auto-c-source'");
    }

    let (program, primary) =
        load_program_with_imports(source, input_path, diagnostics, &options.lib_dirs);
    report_diagnostics(
        input_path,
        source,
        &loi::check_with_warnings_from(&program, &primary),
        diagnostics,
    );
    let ir = match loi::generate_llvm_ir(&program) {
        Ok(ir) => ir,
        Err(error) => {
            eprintln!("loi: {}: {error}", input_path.display());
            process::exit(1);
        }
    };

    let output_path = options
        .output
        .clone()
        .unwrap_or_else(|| BuildKind::Exe.default_output(input_path));
    ensure_distinct_output(input_path, &output_path);

    let ir_path = temporary_path("ll");
    if let Err(error) = fs::write(&ir_path, ir) {
        eprintln!("loi: {}: {error}", ir_path.display());
        process::exit(1);
    }

    let compiler = Tool::from_env("CC", "clang");
    let mut command = compiler.command();
    command.arg("-Wno-override-module");
    if let Some(profile) = options.profile {
        command.arg(profile.cc_flag());
    }
    command.arg("-x").arg("ir").arg(&ir_path);
    // Reset the input language so user-supplied object or source files in
    // --cc-flag are not parsed as LLVM IR.
    command.arg("-x").arg("none");
    command.arg("-o").arg(&output_path);
    for flag in &options.cc_flags {
        command.arg(flag);
    }
    command.arg("-lm");
    let success = run_tool(&compiler, "LLVM compiler", &mut command);
    let _ = fs::remove_file(&ir_path);
    if !success {
        process::exit(1);
    }
}

fn build_source(
    paths: &[PathBuf],
    primary_path: &Path,
    request: BuildRequest<'_>,
    diagnostics: DiagnosticOptions,
) {
    let BuildRequest {
        output,
        obj_dir,
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
        auto_c_source,
        lib_dirs,
    } = request;

    if (!c_sources.is_empty() || auto_c_source) && kind == BuildKind::Object {
        eprintln!(
            "loi: {} cannot be combined with '--emit obj'",
            link_flag_names(c_sources, auto_c_source)
        );
        process::exit(2);
    }

    let compiler = Tool::from_env("CC", "cc");
    let archiver = (kind == BuildKind::Library).then(|| Tool::from_env("AR", "ar"));
    let profile_flag = profile.map(|p| p.cc_flag());
    let arena_flag = arena_size.map(|size| format!("-DLOI_ARENA_SIZE={size}"));

    // Single-file build with retain/rebuild support uses the stamp-based
    // incremental logic. An explicit object directory opts into the
    // multi-object pipeline even for one source, so build scripts get the same
    // path for one file and many files.
    if paths.len() == 1 && obj_dir.is_none() {
        let input_path = &paths[0];
        let source = match fs::read_to_string(input_path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("loi: {}: {error}", input_path.display());
                process::exit(1);
            }
        };

        let (program, primary) =
            load_program_with_imports(&source, input_path, diagnostics, lib_dirs);
        let c_source = generate_checked_program(
            &program,
            &primary,
            input_path,
            &source,
            source_map,
            diagnostics,
        );
        let extern_flows = collect_extern_flows(&source, input_path, lib_dirs);
        let link_sources = with_auto_c_sources(
            c_sources,
            auto_c_source,
            &extern_flows,
            lib_dirs,
            diagnostics,
        );
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
            &link_sources,
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
            match loi::generate_header_named(&program, None, Some(header_path)) {
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
            match loi::generate_ir(&program) {
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

        let exe_link_context;
        let success = match kind {
            BuildKind::Exe => {
                exe_link_context = LinkContext {
                    extern_flows: extern_flows.clone(),
                    options: diagnostics,
                };
                compile_c(
                    &compiler,
                    profile,
                    cc_flags,
                    arena_flag.as_deref(),
                    &output_path,
                    &c_path,
                    &link_sources,
                    false,
                    Some(&exe_link_context),
                )
            }
            BuildKind::Object => compile_c(
                &compiler,
                profile,
                cc_flags,
                arena_flag.as_deref(),
                &output_path,
                &c_path,
                &[],
                true,
                None,
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
                &link_sources,
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
    // Multi-object build: check the merged program once, then compile one
    // translation unit per input (with imported items folded into the first
    // unit that pulls them in) and link the objects.
    // ------------------------------------------------------------------

    if let Some(dir) = obj_dir
        && let Err(error) = fs::create_dir_all(dir)
    {
        eprintln!("loi: {}: {error}", dir.display());
        process::exit(1);
    }
    let mut object_stems: HashMap<String, usize> = HashMap::new();

    let mut registry = ModuleRegistry::default();
    let mut seen_inputs = HashSet::new();
    let mut roots: Vec<(PathBuf, HashSet<usize>, String)> = Vec::new();
    for input_path in paths {
        if !seen_inputs.insert(canonical_key(input_path)) {
            continue;
        }
        let source = match fs::read_to_string(input_path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("loi: {}: {error}", input_path.display());
                process::exit(1);
            }
        };
        let before = registry.modules.len();
        load_module(
            &source,
            input_path,
            diagnostics,
            lib_dirs,
            &mut registry,
            &mut Vec::new(),
        );
        // Only the unit that first loaded a module defines its symbols. Later
        // roots see the same module as an external prototype, as before.
        let owned = (before..registry.modules.len()).collect::<HashSet<usize>>();
        roots.push((input_path.clone(), owned, source));
    }
    let (merged_items, names_by_module) = resolve_and_flatten(&registry, diagnostics);
    let mut units: Vec<(PathBuf, HashSet<String>, String)> = Vec::new();
    for (input_path, owned, source) in &roots {
        let defined = owned
            .iter()
            .filter_map(|id| names_by_module.get(id))
            .flatten()
            .cloned()
            .collect();
        units.push((input_path.clone(), defined, source.clone()));
    }
    let merged = loi::Program {
        items: merged_items,
    };
    let primary_source = units
        .iter()
        .find(|(input, _, _)| input == primary_path)
        .map(|(_, _, source)| source.as_str())
        .unwrap_or("");
    let primary_id = registry
        .by_path
        .get(&canonical_key(primary_path))
        .copied()
        .unwrap_or(0);
    let primary = loi::Program {
        items: registry.modules[primary_id].items.clone(),
    };
    report_diagnostics(
        primary_path,
        primary_source,
        &loi::check_with_warnings_from(&merged, &primary),
        diagnostics,
    );

    let extern_flows = collect_extern_flows_from_units(&units, lib_dirs);
    let link_sources = with_auto_c_sources(
        c_sources,
        auto_c_source,
        &extern_flows,
        lib_dirs,
        diagnostics,
    );

    let mut object_files = Vec::new();
    for (input_path, defined, _source) in &units {
        let c_source = match loi::generate_c_subset(&merged, source_map, defined) {
            Ok(c_source) => c_source,
            Err(error) => {
                eprintln!("loi: {}: {error}", input_path.display());
                process::exit(1);
            }
        };

        let base_stem = input_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("output")
            .to_string();
        let stem = match obj_dir {
            Some(_) => {
                let count = object_stems.entry(base_stem.clone()).or_insert(0);
                let chosen = if *count == 0 {
                    base_stem
                } else {
                    format!("{base_stem}-{}", *count + 1)
                };
                *count += 1;
                chosen
            }
            None => base_stem,
        };
        let (object_path, header_path) = match obj_dir {
            Some(dir) => (dir.join(format!("{stem}.o")), dir.join(format!("{stem}.h"))),
            None => (
                input_path.with_extension("o"),
                input_path.with_extension("h"),
            ),
        };
        ensure_distinct_output(input_path, &object_path);

        let c_path = temporary_c_path();
        if let Err(error) = fs::write(&c_path, &c_source) {
            eprintln!("loi: {}: {error}", c_path.display());
            process::exit(1);
        }

        ensure_distinct_output(input_path, &header_path);
        match loi::generate_header_named(&merged, None, Some(header_path.as_path())) {
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

    if kind == BuildKind::Library && !link_sources.is_empty() {
        eprintln!(
            "loi: {} is not supported by multi-file library builds",
            link_flag_names(c_sources, auto_c_source)
        );
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
            for source in &link_sources {
                command.arg(source);
            }
            // Linker inputs (notably `-l` flags) must follow the objects.
            for flag in cc_flags {
                command.arg(flag);
            }
            command.arg("-lm");
            let link_context = LinkContext {
                extern_flows: extern_flows.clone(),
                options: diagnostics,
            };
            if !run_c_link(&compiler, &mut command, &link_context) {
                process::exit(1);
            }
        }
        BuildKind::Object => {
            // One object per input was written to the object directory (or
            // beside its source when --obj-dir was not given).
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

/// An `extern flow` declaration emitted by the C backend as a foreign symbol.
#[derive(Clone)]
struct ExternFlow {
    c_symbol: String,
    display_name: String,
    file: String,
    source: String,
    span: loi::Span,
    help: String,
}

/// The data needed to translate linker undefined-symbol errors back into
/// diagnostics on Loi `extern flow` declarations.
struct LinkContext {
    extern_flows: Vec<ExternFlow>,
    options: DiagnosticOptions,
}

/// Collect every `extern flow` in `source` and in the files it imports,
/// keeping the source text and file name of each declaration so diagnostics can
/// point at the right file even when it was imported.
fn collect_extern_flows(source: &str, path: &Path, lib_dirs: &[PathBuf]) -> Vec<ExternFlow> {
    let mut visited = HashSet::new();
    let mut found = Vec::new();
    collect_extern_flows_from(source, path, lib_dirs, &mut visited, &mut found);
    found
}

fn collect_extern_flows_from_units(
    units: &[(PathBuf, HashSet<String>, String)],
    lib_dirs: &[PathBuf],
) -> Vec<ExternFlow> {
    let mut visited = HashSet::new();
    let mut found = Vec::new();
    for (path, _, source) in units {
        collect_extern_flows_from(source, path, lib_dirs, &mut visited, &mut found);
    }
    found
}

fn collect_extern_flows_from(
    source: &str,
    path: &Path,
    lib_dirs: &[PathBuf],
    visited: &mut HashSet<PathBuf>,
    found: &mut Vec<ExternFlow>,
) {
    if !visited.insert(canonical_key(path)) {
        return;
    }
    let Ok(program) = parse_source_program(source, path) else {
        return;
    };
    for item in program.items {
        if let loi::Item::Import(import) = &item {
            // Use the same file resolution as the loader so a logical import
            // cannot hide an `extern flow` from the linker diagnostic.
            if let Ok(targets) = loi::imports::resolve_import_with(import, path, lib_dirs) {
                for target in targets {
                    if let Ok(child_source) = fs::read_to_string(&target.path) {
                        collect_extern_flows_from(
                            &child_source,
                            &target.path,
                            lib_dirs,
                            visited,
                            found,
                        );
                    }
                }
            }
            continue;
        }
        let loi::Item::Function(decl) = &item else {
            continue;
        };
        if !decl.external {
            continue;
        }
        let c_symbol = decl.link_name.clone().unwrap_or_else(|| decl.name.clone());
        // Library bindings spell their foreign flows with a `loi_` prefix to
        // avoid clashing with C names. Show the Loi-facing name to the user.
        let display_name = decl
            .name
            .strip_prefix("loi_")
            .unwrap_or(&decl.name)
            .to_string();
        let help = shim_help(path, &c_symbol)
            .unwrap_or_else(|| "pass the C file that defines it with --c-source FILE".to_string());
        found.push(ExternFlow {
            c_symbol,
            display_name,
            file: path.display().to_string(),
            source: source.to_string(),
            span: decl.span,
            help,
        });
    }
}

/// Find the C shim that defines an `extern flow`'s symbol.
///
/// A flat library keeps `<stem>_shim.c` beside its Loi source. A package keeps
/// its shim at the package root, which may be an ancestor of the file that
/// declares the extern. Both are explicit filesystem conventions: there is no
/// global search path, and a candidate must actually define the C symbol.
fn shim_help(path: &Path, c_symbol: &str) -> Option<String> {
    let mut adjacent = path.file_stem()?.to_os_string();
    adjacent.push("_shim.c");
    let adjacent = path.with_file_name(adjacent);
    let mut fallback = None;
    if adjacent.is_file() {
        if shim_defines(&adjacent, c_symbol) {
            return Some(shim_hint(&adjacent));
        }
        fallback = Some(adjacent);
    }
    for dir in path.ancestors().skip(1) {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut candidates = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|candidate| {
                candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with("_shim.c"))
            })
            .collect::<Vec<_>>();
        candidates.sort();
        for candidate in candidates {
            if shim_defines(&candidate, c_symbol) {
                return Some(shim_hint(&candidate));
            }
        }
    }
    fallback.map(|shim| shim_hint(&shim))
}

fn shim_defines(path: &Path, c_symbol: &str) -> bool {
    fs::read_to_string(path).is_ok_and(|source| c_defines_function(&source, c_symbol))
}

/// True when `source` contains a C function definition for `symbol`.
///
/// Inactive preprocessor branches and `#define` lines are blanked first, then
/// comments and string/character literals. The remaining scan is deliberately
/// small: it recognises `name(...) <decorations> { ... }`, where decorations are
/// attributes or qualifiers between the parameter list and the body. It is
/// enough for vendored shims and conservative enough not to select a shim from
/// prose, a macro or disabled code.
fn c_defines_function(source: &str, symbol: &str) -> bool {
    let sanitized = blank_c_comments_and_literals(&active_c_source(source));
    let bytes = sanitized.as_bytes();
    let needle = symbol.as_bytes();
    if needle.is_empty() {
        return false;
    }
    let mut index = 0;
    while index + needle.len() <= bytes.len() {
        let before_ok = index == 0 || !is_c_ident_byte(bytes[index - 1]);
        let after = index + needle.len();
        let after_ok = after == bytes.len() || !is_c_ident_byte(bytes[after]);
        if before_ok && after_ok && &bytes[index..after] == needle {
            let mut cursor = after;
            skip_c_whitespace(bytes, &mut cursor);
            if skip_c_parens(bytes, &mut cursor) {
                skip_c_decorations(bytes, &mut cursor);
                if bytes.get(cursor) == Some(&b'{') {
                    return true;
                }
            }
        }
        index += 1;
    }
    false
}

fn is_c_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn skip_c_whitespace(bytes: &[u8], cursor: &mut usize) {
    while bytes.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
        *cursor += 1;
    }
}

/// Skip a balanced parenthesised group, leaving `cursor` after the `)`.
fn skip_c_parens(bytes: &[u8], cursor: &mut usize) -> bool {
    if bytes.get(*cursor) != Some(&b'(') {
        return false;
    }
    let mut depth = 0usize;
    while *cursor < bytes.len() {
        match bytes[*cursor] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    *cursor += 1;
                    return true;
                }
            }
            _ => {}
        }
        *cursor += 1;
    }
    false
}

/// Skip a balanced `[[...]]` C attribute, leaving `cursor` after `]]`.
fn skip_c_attributes(bytes: &[u8], cursor: &mut usize) -> bool {
    if bytes.get(*cursor) != Some(&b'[') {
        return false;
    }
    let mut depth = 0usize;
    while let Some(byte) = bytes.get(*cursor) {
        match byte {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    *cursor += 1;
                    return true;
                }
            }
            _ => {}
        }
        *cursor += 1;
    }
    false
}

/// Skip attributes and qualifiers between `)` and `{`.
///
/// An identifier followed by a balanced group (`__attribute__((weak))`) or a
/// `[[...]]` attribute is treated as a decoration; a few known qualifiers are
/// accepted without one. Unknown bare identifiers stop the scan so a later
/// block is not mistaken for this function's body.
fn skip_c_decorations(bytes: &[u8], cursor: &mut usize) {
    loop {
        skip_c_whitespace(bytes, cursor);
        if bytes.get(*cursor) == Some(&b'{') {
            return;
        }
        if bytes.get(*cursor) == Some(&b'[') {
            if !skip_c_attributes(bytes, cursor) {
                return;
            }
            continue;
        }
        let start = *cursor;
        if !bytes
            .get(start)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        {
            return;
        }
        while bytes
            .get(*cursor)
            .is_some_and(|byte| is_c_ident_byte(*byte))
        {
            *cursor += 1;
        }
        let mut probe = *cursor;
        skip_c_whitespace(bytes, &mut probe);
        if bytes.get(probe) == Some(&b'(') {
            *cursor = probe;
            if !skip_c_parens(bytes, cursor) {
                *cursor = start;
                return;
            }
            continue;
        }
        let qualifier = &bytes[start..*cursor];
        if matches!(
            qualifier,
            b"const"
                | b"volatile"
                | b"restrict"
                | b"noexcept"
                | b"throw"
                | b"_Noreturn"
                | b"__cdecl"
                | b"__stdcall"
                | b"__fastcall"
        ) {
            continue;
        }
        *cursor = start;
        return;
    }
}

/// Blank preprocessor lines that cannot contribute a definition.
///
/// `#define` lines are hidden so a function-like macro name is not mistaken for
/// a C function. `#if 0` branches (with nested conditionals and `#else`) are
/// hidden; other conditional branches are kept conservatively because the
/// scanner does not evaluate arbitrary preprocessor expressions.
fn active_c_source(source: &str) -> String {
    #[derive(Clone, Copy)]
    struct CondState {
        parent: bool,
        branch: bool,
        taken: bool,
    }

    fn condition_after(keyword: &str, directive: &str) -> bool {
        let expression = directive
            .strip_prefix(keyword)
            .unwrap_or("")
            .split("//")
            .next()
            .unwrap_or("")
            .trim();
        !expression.is_empty() && expression != "0" && expression != "false"
    }

    let mut out = String::with_capacity(source.len());
    let mut stack: Vec<CondState> = Vec::new();
    let mut active = true;
    for line in source.split_inclusive('\n') {
        let directive = line.trim_start().strip_prefix('#').map(str::trim_start);
        let keyword = directive.and_then(|text| text.split_whitespace().next());
        match keyword {
            Some("if") | Some("ifdef") | Some("ifndef") => {
                let condition = match keyword {
                    Some("if") => directive.is_some_and(|text| condition_after("if", text)),
                    _ => true,
                };
                let branch = active && condition;
                stack.push(CondState {
                    parent: active,
                    branch,
                    taken: branch,
                });
                active = branch;
                out.push_str(&blank_c_line(line));
            }
            Some("elif") => {
                if let Some(state) = stack.last_mut() {
                    let condition = directive.is_some_and(|text| condition_after("elif", text));
                    let branch = state.parent && !state.taken && condition;
                    state.branch = branch;
                    state.taken |= branch;
                    active = state.parent && state.branch;
                }
                out.push_str(&blank_c_line(line));
            }
            Some("else") => {
                if let Some(state) = stack.last_mut() {
                    state.branch = state.parent && !state.taken;
                    state.taken = true;
                    active = state.parent && state.branch;
                }
                out.push_str(&blank_c_line(line));
            }
            Some("endif") => {
                stack.pop();
                active = stack
                    .last()
                    .is_none_or(|state| state.parent && state.branch);
                out.push_str(&blank_c_line(line));
            }
            Some("define") if active => out.push_str(&blank_c_line(line)),
            _ if !active => out.push_str(&blank_c_line(line)),
            _ => out.push_str(line),
        }
    }
    out
}

fn blank_c_line(line: &str) -> String {
    line.chars()
        .map(|c| if c == '\n' { '\n' } else { ' ' })
        .collect()
}

fn blank_c_comments_and_literals(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                out.push_str("  ");
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    out.push(' ');
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                out.push_str("  ");
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    out.push(if bytes[index] == b'\n' { '\n' } else { ' ' });
                    index += 1;
                }
                if index + 1 < bytes.len() {
                    out.push_str("  ");
                    index += 2;
                }
            }
            quote @ (b'"' | b'\'') => {
                out.push(' ');
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        out.push_str("  ");
                        index += 2;
                        continue;
                    }
                    if bytes[index] == quote {
                        out.push(' ');
                        index += 1;
                        break;
                    }
                    out.push(if bytes[index] == b'\n' { '\n' } else { ' ' });
                    index += 1;
                }
            }
            byte => {
                out.push(byte as char);
                index += 1;
            }
        }
    }
    out
}

fn shim_hint(path: &Path) -> String {
    format!(
        "this library ships a C shim: build with --c-source {}",
        path.display()
    )
}

/// Name every CLI flag that requested C sources, in a stable order.
fn link_flag_names(c_sources: &[PathBuf], auto: bool) -> String {
    let mut flags = Vec::new();
    if !c_sources.is_empty() {
        flags.push("'--c-source'");
    }
    if auto {
        flags.push("'--auto-c-source'");
    }
    flags.join(" and ")
}

/// Add shim files discovered from imported packages to explicit `--c-source`
/// inputs. Explicit order is preserved and discovered files are deduplicated by
/// canonical path so the same shim is never compiled twice. The first defining
/// candidate wins for one declaring file (adjacent before package root), but
/// two different packages claiming the same C symbol are an ambiguity error
/// instead of a linker duplicate-symbol failure.
fn with_auto_c_sources(
    explicit: &[PathBuf],
    auto: bool,
    extern_flows: &[ExternFlow],
    lib_dirs: &[PathBuf],
    options: DiagnosticOptions,
) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    let mut seen = HashSet::new();
    for source in explicit {
        let key = canonical_key(source);
        if seen.insert(key) {
            sources.push(source.clone());
        }
    }
    if auto {
        let mut discovered = Vec::new();
        let mut chosen: HashMap<String, PathBuf> = HashMap::new();
        for flow in extern_flows {
            let declaring_file = Path::new(&flow.file);
            let mut selected = None;
            for candidate in loi::imports::shim_candidates_with(declaring_file, lib_dirs) {
                if candidate.is_file() && shim_defines(&candidate, &flow.c_symbol) {
                    selected = Some(candidate);
                    break;
                }
            }
            let Some(candidate) = selected else {
                continue;
            };
            if let Some(existing) = chosen.get(&flow.c_symbol) {
                if canonical_key(existing) != canonical_key(&candidate) {
                    let candidates = [
                        existing.display().to_string(),
                        candidate.display().to_string(),
                    ]
                    .join(", ");
                    let diagnostic = loi::Diagnostic::from_error(
                        format!(
                            "ambiguous C shim for extern flow '{}' (C symbol '{}')",
                            flow.display_name, flow.c_symbol
                        ),
                        flow.span,
                    )
                    .with_label("two imported packages define this symbol")
                    .with_help(format!(
                        "candidates: {candidates}; import one package or pass --c-source FILE explicitly"
                    ));
                    report_one_diagnostic(Path::new(&flow.file), &flow.source, diagnostic, options);
                }
            } else {
                chosen.insert(flow.c_symbol.clone(), candidate.clone());
            }
            discovered.push(candidate);
        }
        discovered.sort();
        for candidate in discovered {
            let key = canonical_key(&candidate);
            if seen.insert(key) {
                sources.push(candidate);
            }
        }
    }
    sources
}

/// The symbol named by a linker "undefined reference"/"undefined symbol" line.
fn undefined_symbol(line: &str) -> Option<&str> {
    if let Some((_, rest)) = line.split_once("undefined reference to `") {
        return rest.split_once('\'').map(|(symbol, _)| symbol);
    }
    if let Some((_, rest)) = line.split_once("undefined symbol: ") {
        return rest
            .split_whitespace()
            .next()
            .map(|symbol| symbol.trim_start_matches('_'));
    }
    None
}

/// Turn undefined foreign symbols into source diagnostics, then print the
/// remaining linker output verbatim. Unknown symbols keep the raw behavior.
fn report_link_failure(stderr: &str, context: &LinkContext) {
    let mut sourced = Vec::new();
    let mut remaining = Vec::new();
    let mut mapped_symbols = HashSet::new();
    for line in stderr.lines() {
        let flow = undefined_symbol(line).and_then(|symbol| {
            context
                .extern_flows
                .iter()
                .find(|flow| flow.c_symbol == symbol)
        });
        if let Some(flow) = flow {
            if !mapped_symbols.insert(flow.c_symbol.as_str()) {
                continue;
            }
            let diagnostic = loi::Diagnostic::error(
                format!(
                    "extern flow '{}' (C symbol '{}') is not defined by any linked C source",
                    flow.display_name, flow.c_symbol
                ),
                flow.span,
            )
            .with_help(flow.help.clone());
            sourced.push(loi::diagnostic::SourcedDiagnostic {
                diagnostic,
                source: flow.source.clone(),
                file: flow.file.clone(),
            });
        } else {
            remaining.push(line);
        }
    }

    if sourced.is_empty() {
        eprint!("{stderr}");
        return;
    }
    eprint!(
        "{}",
        loi::diagnostic::render_sourced(
            &sourced,
            context.options.format,
            context.options.color.resolve()
        )
    );
    if !remaining.is_empty() {
        eprintln!("{}", remaining.join("\n"));
    }
}

/// Run the linker with captured output so undefined `extern flow` symbols can
/// be explained at their Loi declaration instead of surfacing as raw `ld` text.
fn run_c_link(compiler: &Tool, command: &mut Command, context: &LinkContext) -> bool {
    use std::io::Write as _;

    match command.output() {
        Ok(output) => {
            let _ = std::io::stdout().write_all(&output.stdout);
            if output.status.success() {
                let _ = std::io::stderr().write_all(&output.stderr);
                true
            } else {
                report_link_failure(&String::from_utf8_lossy(&output.stderr), context);
                false
            }
        }
        Err(error) => {
            eprintln!(
                "loi: failed to run C compiler/linker '{}': {error}",
                compiler.program.to_string_lossy()
            );
            false
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
    link_context: Option<&LinkContext>,
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
    match link_context {
        Some(context) if !object_only => run_c_link(compiler, &mut command, context),
        _ => run_tool(compiler, "C compiler", &mut command),
    }
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
        None,
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
                None,
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

fn load_program(source: &str, path: &Path, diagnostics: DiagnosticOptions) -> loi::Program {
    match parse_source_program(source, path) {
        Ok(program) => program,
        Err(error) => report_one_diagnostic(
            path,
            source,
            loi::Diagnostic::from_error(
                error.message,
                frontend_span(error.line, error.column, error.end_column),
            ),
            diagnostics,
        ),
    }
}

/// A frontend span with an exact end for lex errors. Parser errors keep the
/// line-scan fallback used by the human renderer until they carry their own end.
fn frontend_span(line: usize, column: usize, end_column: Option<usize>) -> loi::Span {
    let span = loi::Span::new(line, column);
    match end_column {
        Some(end) => span.with_end_column(end),
        None => span,
    }
}

/// The position and message of the first lexer or parser error in one file.
/// Kept small because it is the `Err` type of the frontend pipeline.
struct FrontendError {
    message: String,
    line: usize,
    column: usize,
    end_column: Option<usize>,
}

/// Lex and parse one file, keeping the source position of the first frontend
/// error so the CLI can render it like every checker diagnostic.
fn parse_source_program(source: &str, path: &Path) -> Result<loi::Program, FrontendError> {
    let tokens =
        loi::tokenize(source, &path.display().to_string()).map_err(|error| FrontendError {
            message: error.message,
            line: error.line,
            column: error.column,
            end_column: Some(error.end_column),
        })?;
    loi::parse(tokens).map_err(|error| FrontendError {
        message: error.message,
        line: error.line,
        column: error.column,
        end_column: None,
    })
}

fn canonical_key(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// One loaded source file. Imports keep their aliases so the resolver can
/// tell which module a qualified name refers to.
struct LoadedModule {
    name: String,
    path: PathBuf,
    source: String,
    items: Vec<loi::Item>,
    /// `(alias, module id, re-exports its public items)` for each import.
    imports: Vec<(String, usize, bool)>,
}

#[derive(Default)]
struct ModuleRegistry {
    modules: Vec<LoadedModule>,
    by_path: HashMap<PathBuf, usize>,
}

fn module_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("module")
        .to_string()
}

/// Parse one file and recursively load its imports, registering each file once
/// by canonical path. `stack` holds the files currently being expanded, so a
/// re-import from an ancestor is reported as a cycle rather than deduplicated.
fn load_module(
    source: &str,
    path: &Path,
    diagnostics: DiagnosticOptions,
    lib_dirs: &[PathBuf],
    registry: &mut ModuleRegistry,
    stack: &mut Vec<PathBuf>,
) -> usize {
    let key = canonical_key(path);
    if let Some(&id) = registry.by_path.get(&key) {
        return id;
    }
    let program = load_program(source, path, diagnostics);
    let id = registry.modules.len();
    registry.by_path.insert(key.clone(), id);
    registry.modules.push(LoadedModule {
        name: module_name(path),
        path: path.to_path_buf(),
        source: source.to_string(),
        items: program.items,
        imports: Vec::new(),
    });

    let imports = registry.modules[id]
        .items
        .iter()
        .filter_map(|item| match item {
            loi::Item::Import(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut aliases = HashMap::new();
    aliases.insert(module_name(path), id);
    for import in imports {
        let targets = match loi::imports::resolve_import_with(&import, path, lib_dirs) {
            Ok(targets) => targets,
            Err(error) => report_one_diagnostic(path, source, error.into_diagnostic(), diagnostics),
        };
        for target in targets {
            let child = target.path;
            let child_key = canonical_key(&child);
            if stack.contains(&child_key) {
                report_one_diagnostic(
                    path,
                    source,
                    loi::Diagnostic::from_error(
                        format!("import cycle detected while loading '{}'", child.display()),
                        target.declaration_span,
                    )
                    .with_label("this import returns to a file already being loaded"),
                    diagnostics,
                );
            }
            let child_source = match fs::read_to_string(&child) {
                Ok(source) => source,
                Err(error) => {
                    eprintln!("loi: {}: {error}", child.display());
                    process::exit(1);
                }
            };
            stack.push(key.clone());
            let module = load_module(
                &child_source,
                &child,
                diagnostics,
                lib_dirs,
                registry,
                stack,
            );
            stack.pop();
            if let Some(&existing) = aliases.get(&target.alias)
                && existing != module
            {
                report_one_diagnostic(
                    path,
                    source,
                    loi::Diagnostic::from_error(
                        format!("duplicate module alias '{}'", target.alias),
                        target.alias_span,
                    )
                    .with_label("this alias is already bound in this module"),
                    diagnostics,
                );
            }
            aliases.insert(target.alias.clone(), module);
            registry.modules[id]
                .imports
                .push((target.alias, module, target.public));
        }
    }
    id
}

/// Build the alias table for each module and the global declaration index used
/// by the resolver. Duplicate declaration names are kept so the checker can
/// report them; sealed checks only apply when a name has a single declaration.
fn build_resolution_context(registry: &ModuleRegistry) -> (Vec<ModuleView>, ResolutionContext) {
    let mut views = registry
        .modules
        .iter()
        .map(|module| ModuleView {
            name: module.name.clone(),
            aliases: HashMap::new(),
        })
        .collect::<Vec<_>>();
    for (id, module) in registry.modules.iter().enumerate() {
        views[id].aliases.insert(module.name.clone(), id);
        for (alias, target, _) in &module.imports {
            views[id].aliases.insert(alias.clone(), *target);
        }
    }

    // Direct declarations are the only definitions. Re-exports add lookup
    // entries to the importing module without creating a second declaration or
    // changing the C symbol owned by the defining module.
    let mut direct: Vec<Vec<ModuleExport>> = vec![Vec::new(); registry.modules.len()];
    for (id, module) in registry.modules.iter().enumerate() {
        for item in &module.items {
            if let Some(name) = item.declared_name() {
                direct[id].push(ModuleExport {
                    module: id,
                    owner: id,
                    name: name.to_string(),
                    visibility: item.visibility(),
                    span: item.span(),
                    return_type: match item {
                        loi::Item::Function(decl) => match decl.return_type.as_ref() {
                            Some(loi::ast::Type::Named(name, _)) => {
                                Some(name.rsplit("::").next().unwrap_or(name).to_string())
                            }
                            _ => None,
                        },
                        _ => None,
                    },
                });
            }
        }
    }

    // Follow `open import` transitively so a package root can re-export another
    // package root while keeping the defining file as the single owner.
    let mut reexports: Vec<HashSet<usize>> = vec![HashSet::new(); registry.modules.len()];
    for (id, module) in registry.modules.iter().enumerate() {
        let mut reachable = HashSet::new();
        let mut work = module
            .imports
            .iter()
            .filter(|(_, _, public)| *public)
            .map(|(_, target, _)| *target)
            .collect::<Vec<_>>();
        while let Some(target) = work.pop() {
            if target == id || !reachable.insert(target) {
                continue;
            }
            for (_, nested, public) in &registry.modules[target].imports {
                if *public {
                    work.push(*nested);
                }
            }
        }
        reexports[id] = reachable;
    }

    let mut context = ResolutionContext::default();
    for (id, entries) in direct.iter().enumerate() {
        for export in entries {
            context
                .exports
                .entry(export.name.clone())
                .or_default()
                .push(export.clone());
        }
        for owner in &reexports[id] {
            for export in &direct[*owner] {
                if export.visibility == loi::ast::Visibility::Visible {
                    context
                        .exports
                        .entry(export.name.clone())
                        .or_default()
                        .push(ModuleExport {
                            module: id,
                            owner: *owner,
                            name: export.name.clone(),
                            visibility: export.visibility,
                            span: export.span,
                            return_type: export.return_type.clone(),
                        });
                }
            }
        }
    }
    (views, context)
}

/// Resolve every module's references and flatten the items into the single
/// program the checker and backend consume. Also returns the names each module
/// contributes, which multi-file builds use to decide what to define.
fn resolve_and_flatten(
    registry: &ModuleRegistry,
    diagnostics: DiagnosticOptions,
) -> (Vec<loi::Item>, HashMap<usize, Vec<String>>) {
    let (views, context) = build_resolution_context(registry);
    let mut items = Vec::new();
    let mut names = HashMap::new();
    for (id, module) in registry.modules.iter().enumerate() {
        let mut module_items = module.items.clone();
        if let Some(error) = loi::modules::resolve_items(&mut module_items, id, &views, &context)
            .into_iter()
            .next()
        {
            report_one_diagnostic(&module.path, &module.source, error, diagnostics);
        }
        for item in module_items {
            if let loi::Item::Import(_) = item {
                continue;
            }
            if let Some(name) = item.declared_name() {
                names
                    .entry(id)
                    .or_insert_with(Vec::new)
                    .push(name.to_string());
            }
            items.push(item);
        }
    }
    (items, names)
}

/// Load a source file together with everything it imports. The first program
/// is the flattened unit the checker and backend consume; the second is the
/// primary file's own AST, which is the only source lint warnings may quote.
fn load_program_with_imports(
    source: &str,
    path: &Path,
    diagnostics: DiagnosticOptions,
    lib_dirs: &[PathBuf],
) -> (loi::Program, loi::Program) {
    let mut registry = ModuleRegistry::default();
    let primary_id = load_module(
        source,
        path,
        diagnostics,
        lib_dirs,
        &mut registry,
        &mut Vec::new(),
    );
    let (items, _) = resolve_and_flatten(&registry, diagnostics);
    let primary = loi::Program {
        items: registry.modules[primary_id].items.clone(),
    };
    (loi::Program { items }, primary)
}

fn generate_checked_program(
    program: &loi::Program,
    primary: &loi::Program,
    path: &Path,
    source: &str,
    source_map_path: Option<&Path>,
    diagnostics: DiagnosticOptions,
) -> String {
    report_diagnostics(
        path,
        source,
        &loi::check_with_warnings_from(program, primary),
        diagnostics,
    );

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

#[derive(Clone, Copy)]
struct DiagnosticOptions {
    format: ErrorFormat,
    color: ColorChoice,
}

/// Render a single frontend error and stop, using the same format and colors
/// as checker diagnostics.
fn report_one_diagnostic(
    path: &Path,
    source: &str,
    diagnostic: loi::Diagnostic,
    options: DiagnosticOptions,
) -> ! {
    let color = options.color.resolve();
    let rendered = loi::diagnostic::render_all(
        &[diagnostic],
        source,
        &path.display().to_string(),
        options.format,
        color,
    );
    eprint!("{rendered}");
    process::exit(1);
}

fn report_diagnostics(
    path: &Path,
    source: &str,
    diagnostics: &[loi::Diagnostic],
    options: DiagnosticOptions,
) {
    // A clean JSON check is still a structured result (`[]`), so clients do not
    // have to special-case empty stderr. Human and short formats stay silent.
    if diagnostics.is_empty() && options.format != loi::ErrorFormat::Json {
        return;
    }

    // Warnings are still printed (the LSP reads stderr for structured
    // diagnostics), but only errors make the compiler exit unsuccessfully.
    let has_errors = diagnostics.iter().any(|diagnostic| diagnostic.is_error());
    let color = options.color.resolve();
    let rendered = loi::diagnostic::render_all(
        diagnostics,
        source,
        &path.display().to_string(),
        options.format,
        color,
    );
    eprint!("{rendered}");
    if has_errors {
        process::exit(1);
    }
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
