use loi::{check, format_program_with_comments, parse, tokenize_with_comments};

/// Normalize the HTML entities used inside `<code>` blocks so the same checks
/// work on rendered snippets and raw JavaScript template literals.
fn normalize(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

/// Legacy shape literals and declarations are spelled `Name[ ... ]` and
/// `shape Name [ ... ]`. Braces are canonical; brackets belong to slices and
/// indexing only.
fn has_legacy_bracket_shape(text: &str) -> bool {
    let bytes = text.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] != b'[' || index == 0 {
            continue;
        }
        let mut start = index;
        while start > 0
            && (bytes[start - 1].is_ascii_alphanumeric()
                || bytes[start - 1] == b'_'
                || bytes[start - 1] == b'>')
        {
            start -= 1;
        }
        if start < index && bytes[start].is_ascii_uppercase() {
            return true;
        }
    }
    false
}

fn website_documentation_files() -> &'static [&'static str] {
    &[
        "website/index.html",
        "website/docs.html",
        "website/playground.html",
        "website/404.html",
        "website/README.md",
        "website/assets/playground.js",
    ]
}

#[test]
fn website_snippets_use_canonical_shape_syntax() {
    for path in website_documentation_files() {
        let text = match *path {
            "website/index.html" => include_str!("../website/index.html").to_string(),
            "website/docs.html" => include_str!("../website/docs.html").to_string(),
            "website/playground.html" => include_str!("../website/playground.html").to_string(),
            "website/404.html" => include_str!("../website/404.html").to_string(),
            "website/README.md" => include_str!("../website/README.md").to_string(),
            "website/assets/playground.js" => {
                include_str!("../website/assets/playground.js").to_string()
            }
            other => panic!("unlisted website file: {other}"),
        };
        // JavaScript files also contain object lookups such as `PRESETS[name]`;
        // only their embedded Loi programs are part of the documented surface.
        let code = if path.ends_with(".js") {
            playground_presets().join("\n")
        } else {
            text.clone()
        };
        let normalized = normalize(&code);
        assert!(
            !has_legacy_bracket_shape(&normalized),
            "{path} still contains a bracketed shape declaration or literal"
        );
        let normalized = normalize(&text);
        assert!(
            !normalized.contains("No brace shape literals"),
            "{path} still claims brace shape literals do not exist"
        );
        assert!(
            !normalized.contains("[]T{"),
            "{path} still spells slice literals as []T{{ ... }}"
        );
        assert!(
            !normalized.contains("out/inout parameter"),
            "{path} still uses the old out/inout spelling"
        );
    }
}

/// Extract the inline starter programs from the playground preset table.
fn playground_presets() -> Vec<String> {
    let source = include_str!("../website/assets/playground.js");
    let marker = "code: `";
    let mut rest = source;
    let mut presets = Vec::new();
    while let Some(start) = rest.find(marker) {
        rest = &rest[start + marker.len()..];
        let Some(end) = rest.find("`,") else {
            break;
        };
        // The JavaScript template literal escapes Loi's `\\n` as `\\\\n`.
        presets.push(rest[..end].replace("\\\\", "\\"));
        rest = &rest[end + 2..];
    }
    presets
}

#[test]
fn playground_presets_are_canonical_and_clean() {
    let presets = playground_presets();
    assert_eq!(
        presets.len(),
        3,
        "expected hello/directions/ownership presets"
    );
    for (index, source) in presets.iter().enumerate() {
        let (tokens, comments) = tokenize_with_comments(source, "<preset>")
            .unwrap_or_else(|error| panic!("preset {index} does not lex: {error:?}"));
        let program = parse(tokens)
            .unwrap_or_else(|error| panic!("preset {index} does not parse: {error:?}"));
        let diagnostics = check(&program);
        assert!(
            diagnostics.is_empty(),
            "preset {index} has diagnostics: {diagnostics:?}"
        );
        let formatted = format_program_with_comments(&program, &comments);
        assert_eq!(
            &formatted, source,
            "preset {index} is not canonically formatted"
        );
    }
}

#[test]
fn release_version_is_0_2_0_everywhere() {
    const VERSION: &str = "0.2.0";
    assert_eq!(
        env!("CARGO_PKG_VERSION"),
        VERSION,
        "the CLI prints CARGO_PKG_VERSION and cannot declare another release"
    );
    let live_files = [
        ("Cargo.toml", include_str!("../Cargo.toml")),
        ("Cargo.lock", include_str!("../Cargo.lock")),
        (
            "website/package.json",
            include_str!("../website/package.json"),
        ),
        (
            "editors/vscode-loi/package.json",
            include_str!("../editors/vscode-loi/package.json"),
        ),
        (
            "editors/vscode-loi/package-lock.json",
            include_str!("../editors/vscode-loi/package-lock.json"),
        ),
        ("website/index.html", include_str!("../website/index.html")),
        ("website/docs.html", include_str!("../website/docs.html")),
        (
            "website/playground.html",
            include_str!("../website/playground.html"),
        ),
        ("website/404.html", include_str!("../website/404.html")),
        (
            "benchmark/results.json",
            include_str!("../benchmark/results.json"),
        ),
        (
            "development/FINAL_REPORT.md",
            include_str!("../development/FINAL_REPORT.md"),
        ),
        (
            "development/PROBES.md",
            include_str!("../development/PROBES.md"),
        ),
        (
            "development/PROGRESS.md",
            include_str!("../development/PROGRESS.md"),
        ),
    ];
    for (path, text) in live_files {
        assert!(
            text.contains(VERSION),
            "{path} does not declare release {VERSION}"
        );
        assert!(
            !text.contains("0.1.0"),
            "{path} still declares the superseded 0.1.0"
        );
    }
    for path in [
        "website/index.html",
        "website/docs.html",
        "website/playground.html",
        "website/404.html",
    ] {
        let text = match path {
            "website/index.html" => include_str!("../website/index.html"),
            "website/docs.html" => include_str!("../website/docs.html"),
            "website/playground.html" => include_str!("../website/playground.html"),
            "website/404.html" => include_str!("../website/404.html"),
            other => panic!("unlisted HTML surface: {other}"),
        };
        assert!(text.contains("v0.2"), "{path} still shows the v0.1 badge");
        assert!(!text.contains("v0.1"), "{path} still shows the v0.1 badge");
    }
    assert!(
        include_str!("../website/docs.html").contains("loi-lsp-0.2.0.vsix"),
        "the VS Code install example still names the old VSIX"
    );
}
