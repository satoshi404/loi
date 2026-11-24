//! Rich diagnostic rendering.
//!
//! The checker keeps producing the same messages; this module only controls how
//! they are shown. [`Diagnostic::from_error`] is the lossless conversion from
//! the old `(message, span)` errors, and the builder methods add labels, help
//! and notes without changing the message text.
//!
//! Rendering is deliberately dependency-free: colors are plain ANSI escapes and
//! `--error-format=json` writes valid JSON by hand.

use std::env;
use std::fmt::Write as _;
use std::io::IsTerminal;

use crate::ast::Span;

/// Diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
        }
    }

    fn color(self) -> &'static str {
        match self {
            Self::Error => RED,
            Self::Warning => YELLOW,
            Self::Note => CYAN,
        }
    }
}

/// How to decide whether ANSI colors are emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorChoice {
    Auto,
    #[default]
    Always,
    Never,
}

impl ColorChoice {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    /// Resolve the choice for a compiler writing to stderr.
    ///
    /// Loi emits ANSI colors by default. `--color never` and a non-empty
    /// `NO_COLOR` both turn them off; `CLICOLOR_FORCE` and a terminal only
    /// matter for the opt-in `--color auto` mode.
    pub fn resolve(self) -> bool {
        if env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
            return false;
        }
        match self {
            Self::Never => false,
            Self::Always => true,
            Self::Auto => {
                if env::var_os("CLICOLOR_FORCE").is_some_and(|value| value != "0") {
                    return true;
                }
                std::io::stderr().is_terminal()
            }
        }
    }
}

/// Output format for compiler diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorFormat {
    #[default]
    Human,
    Short,
    Json,
}

impl ErrorFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "human" => Some(Self::Human),
            "short" => Some(Self::Short),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// A secondary source location with its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecondaryLabel {
    pub span: Span,
    pub label: String,
}

/// One compiler diagnostic.
///
/// `span` is the primary location. `label` is attached to that location when
/// the renderer underlines it. Existing code can keep using `span` and `message`
/// directly, which is why those field names are unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Span,
    pub label: Option<String>,
    pub secondary: Vec<SecondaryLabel>,
    pub help: Option<String>,
    pub note: Option<String>,
}

impl Diagnostic {
    /// The old checker constructor: `Diagnostic::new(span, message)`.
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self::from_error(message, span)
    }

    /// Losslessly convert an existing `(message, span)` error.
    pub fn from_error(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span,
            label: None,
            secondary: Vec::new(),
            help: None,
            note: None,
        }
    }

    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Self::from_error(message, span)
    }

    pub fn warning(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::from_error(message, span)
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_secondary(mut self, span: Span, label: impl Into<String>) -> Self {
        self.secondary.push(SecondaryLabel {
            span,
            label: label.into(),
        });
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    pub fn position(&self) -> (usize, usize) {
        (self.span.line, self.span.column)
    }
}

const RESET: &str = "\x1b[0m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const CYAN: &str = "\x1b[36m";

/// Render a batch of diagnostics in the requested format.
pub fn render_all(
    diagnostics: &[Diagnostic],
    source: &str,
    file: &str,
    format: ErrorFormat,
    color: bool,
) -> String {
    match format {
        ErrorFormat::Human => render_human(diagnostics, source, file, color),
        ErrorFormat::Short => render_short(diagnostics, file),
        ErrorFormat::Json => render_json(diagnostics, file),
    }
}

/// Render one diagnostic without the trailing summary.
pub fn render_one(diagnostic: &Diagnostic, source: &str, file: &str, color: bool) -> String {
    let painter = Painter { enabled: color };
    let width = gutter_width(diagnostic);
    let mut out = String::new();

    let severity = diagnostic.severity;
    let _ = writeln!(
        out,
        "{}: {}",
        painter.paint(severity.color(), severity.as_str()),
        diagnostic.message
    );

    let arrow = painter.paint(BLUE, "-->");
    let _ = writeln!(
        out,
        "{:>width$}{} {}:{}:{}",
        "",
        arrow,
        file,
        diagnostic.span.line,
        diagnostic.span.column,
        width = width
    );

    let gutter = blank_gutter(width, &painter);
    out.push_str(&gutter);
    out.push('\n');

    let mut labels = Vec::new();
    labels.push(LabelRef {
        span: diagnostic.span,
        text: diagnostic.label.as_deref(),
        primary: true,
    });
    for secondary in &diagnostic.secondary {
        labels.push(LabelRef {
            span: secondary.span,
            text: Some(secondary.label.as_str()),
            primary: false,
        });
    }
    labels.sort_by_key(|label| (label.span.line, label.span.column, !label.primary));

    let lines = source.lines().collect::<Vec<_>>();
    let mut current_line = None;
    for label in labels {
        if label.span.line == 0 {
            continue;
        }
        if current_line != Some(label.span.line) {
            current_line = Some(label.span.line);
            let text = lines
                .get(label.span.line.saturating_sub(1))
                .copied()
                .unwrap_or("");
            let _ = writeln!(
                out,
                "{:>width$} {} {}",
                label.span.line,
                painter.paint(BLUE, "|"),
                text,
                width = width
            );
        }
        let text = lines
            .get(label.span.line.saturating_sub(1))
            .copied()
            .unwrap_or("");
        let padding = underline_padding(text, label.span.column);
        let marker = if label.primary { '^' } else { '-' };
        let underline = underline_for(text, label.span, marker);
        let underline = painter.paint(severity.color(), &underline);
        match label.text.filter(|text| !text.is_empty()) {
            Some(text) => {
                let _ = writeln!(
                    out,
                    "{} {}{} {}",
                    blank_gutter(width, &painter),
                    padding,
                    underline,
                    text
                );
            }
            None => {
                let _ = writeln!(
                    out,
                    "{} {}{}",
                    blank_gutter(width, &painter),
                    padding,
                    underline
                );
            }
        }
    }

    if diagnostic.help.is_some() || diagnostic.note.is_some() {
        out.push_str(&blank_gutter(width, &painter));
        out.push('\n');
        if let Some(note) = &diagnostic.note {
            let _ = writeln!(
                out,
                "{}= {}: {}",
                " ".repeat(width + 1),
                painter.paint(CYAN, "note"),
                note
            );
        }
        if let Some(help) = &diagnostic.help {
            let _ = writeln!(
                out,
                "{}= {}: {}",
                " ".repeat(width + 1),
                painter.paint(CYAN, "help"),
                help
            );
        }
    }

    out
}

fn render_human(diagnostics: &[Diagnostic], source: &str, file: &str, color: bool) -> String {
    let sorted = sorted(diagnostics);
    if sorted.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for (index, diagnostic) in sorted.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&render_one(diagnostic, source, file, color));
    }

    let summary = summary(&sorted);
    if !summary.is_empty() {
        out.push('\n');
        out.push_str(&summary);
        out.push('\n');
    }
    out
}

fn render_short(diagnostics: &[Diagnostic], file: &str) -> String {
    let mut out = String::new();
    for diagnostic in sorted(diagnostics) {
        let _ = writeln!(
            out,
            "{}:{}:{}: {}",
            file, diagnostic.span.line, diagnostic.span.column, diagnostic.message
        );
    }
    out
}

fn render_json(diagnostics: &[Diagnostic], file: &str) -> String {
    let sorted = sorted(diagnostics);
    let mut out = String::from("[");
    for (index, diagnostic) in sorted.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        write_json_object(&mut out, diagnostic, file);
    }
    if !sorted.is_empty() {
        out.push('\n');
    }
    out.push_str("]\n");
    out
}

fn write_json_object(out: &mut String, diagnostic: &Diagnostic, file: &str) {
    let _ = write!(
        out,
        "\n  {{\"severity\":\"{}\",\"message\":\"{}\",\"file\":\"{}\",\"line\":{},\"column\":{},\"end_line\":{},\"end_column\":{}",
        diagnostic.severity.as_str(),
        json_escape(&diagnostic.message),
        json_escape(file),
        diagnostic.span.line,
        diagnostic.span.column,
        diagnostic.span.end_line(),
        diagnostic.span.end_column()
    );
    if let Some(label) = &diagnostic.label {
        let _ = write!(out, ",\"label\":\"{}\"", json_escape(label));
    }
    if !diagnostic.secondary.is_empty() {
        out.push_str(",\"secondary\":[");
        for (secondary_index, secondary) in diagnostic.secondary.iter().enumerate() {
            if secondary_index > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"file\":\"{}\",\"line\":{},\"column\":{},\"end_line\":{},\"end_column\":{},\"label\":\"{}\"}}",
                json_escape(file),
                secondary.span.line,
                secondary.span.column,
                secondary.span.end_line(),
                secondary.span.end_column(),
                json_escape(&secondary.label)
            );
        }
        out.push(']');
    }
    if let Some(help) = &diagnostic.help {
        let _ = write!(out, ",\"help\":\"{}\"", json_escape(help));
    }
    if let Some(note) = &diagnostic.note {
        let _ = write!(out, ",\"note\":\"{}\"", json_escape(note));
    }
    out.push('}');
}

/// A diagnostic together with the source text it refers to. CLI-only
/// diagnostics (for example an unresolved `extern flow`) can point into a
/// different file than the one named on the command line.
#[derive(Debug, Clone)]
pub struct SourcedDiagnostic {
    pub diagnostic: Diagnostic,
    pub source: String,
    pub file: String,
}

/// Render diagnostics that may come from different files.
///
/// This mirrors [`render_all`] except that the short format also prints each
/// diagnostic's help line. That is reserved for CLI-only diagnostics, where
/// there is no `.expected` corpus to keep unchanged; language diagnostics keep
/// the compact one-line short form.
pub fn render_sourced(items: &[SourcedDiagnostic], format: ErrorFormat, color: bool) -> String {
    if items.is_empty() {
        return String::new();
    }
    match format {
        ErrorFormat::Human => {
            let mut sorted = items.iter().collect::<Vec<_>>();
            sorted.sort_by_key(|item| item.diagnostic.position());
            let mut out = String::new();
            for (index, item) in sorted.iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                out.push_str(&render_one(
                    &item.diagnostic,
                    &item.source,
                    &item.file,
                    color,
                ));
            }
            let refs = sorted
                .iter()
                .map(|item| &item.diagnostic)
                .collect::<Vec<_>>();
            let summary = summary(&refs);
            if !summary.is_empty() {
                out.push('\n');
                out.push_str(&summary);
                out.push('\n');
            }
            out
        }
        ErrorFormat::Short => {
            let mut out = String::new();
            for item in items {
                let diagnostic = &item.diagnostic;
                let _ = writeln!(
                    out,
                    "{}:{}:{}: {}",
                    item.file, diagnostic.span.line, diagnostic.span.column, diagnostic.message
                );
                if let Some(help) = &diagnostic.help {
                    let _ = writeln!(out, "  = help: {help}");
                }
            }
            out
        }
        ErrorFormat::Json => {
            let mut out = String::from("[");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_json_object(&mut out, &item.diagnostic, &item.file);
            }
            out.push('\n');
            out.push_str("]\n");
            out
        }
    }
}

fn summary(diagnostics: &[&Diagnostic]) -> String {
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .count();
    let warnings = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Warning)
        .count();
    let mut parts = Vec::new();
    if errors > 0 {
        parts.push(format!("{errors} error{}", plural(errors)));
    }
    if warnings > 0 {
        parts.push(format!("{warnings} warning{}", plural(warnings)));
    }
    parts.join(", ")
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn sorted(diagnostics: &[Diagnostic]) -> Vec<&Diagnostic> {
    let mut sorted = diagnostics.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|diagnostic| diagnostic.position());
    sorted
}

fn gutter_width(diagnostic: &Diagnostic) -> usize {
    let mut max_line = diagnostic.span.line;
    for secondary in &diagnostic.secondary {
        max_line = max_line.max(secondary.span.line);
    }
    max_line.to_string().len().max(1)
}

fn blank_gutter(width: usize, painter: &Painter) -> String {
    format!("{}{}", " ".repeat(width + 1), painter.paint(BLUE, "|"))
}

struct LabelRef<'a> {
    span: Span,
    text: Option<&'a str>,
    primary: bool,
}

struct Painter {
    enabled: bool,
}

impl Painter {
    fn paint(&self, color: &str, text: &str) -> String {
        if self.enabled {
            format!("{color}{text}{RESET}")
        } else {
            text.to_string()
        }
    }
}

/// Number of characters to underline for `span` on `line`.
///
/// Parser-produced spans carry an exact end column. For synthetic or imported
/// spans without one, recover the extent by scanning the source line.
fn label_extent(line: &str, span: Span) -> usize {
    if span.end_line() == span.line
        && let Some(end) = span.end_column
    {
        return end.saturating_sub(span.column).max(1);
    }
    token_length(line, span.column)
}

/// Underline a span on its first line. Multi-line spans stop at the end of
/// that line and get an ellipsis; the count is in characters so UTF-8 text and
/// tabs keep their visual alignment through `underline_padding`.
fn underline_for(line: &str, span: Span, marker: char) -> String {
    if span.end_line() > span.line {
        let first_line_length = line.chars().count();
        let available = first_line_length
            .saturating_sub(span.column.saturating_sub(1))
            .max(1);
        return format!("{}{}", marker.to_string().repeat(available), "...");
    }
    marker.to_string().repeat(label_extent(line, span))
}

/// Scan `line` starting at the 1-based `column` and return the token length.
///
/// `pub(crate)` so the lexer can give lex errors the same exclusive end column
/// the human renderer derives; human and JSON ranges must stay in agreement.
pub(crate) fn token_length(line: &str, column: usize) -> usize {
    let chars = line.chars().collect::<Vec<_>>();
    let start = column.saturating_sub(1);
    if start >= chars.len() {
        return 1;
    }
    let first = chars[start];
    if first == '"' {
        let mut index = start + 1;
        while index < chars.len() {
            match chars[index] {
                '\\' => index += 2,
                '"' => {
                    index += 1;
                    break;
                }
                _ => index += 1,
            }
        }
        return (index - start).max(1).min(chars.len() - start);
    }
    if first.is_alphabetic() || first == '_' {
        let mut index = start;
        while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
            index += 1;
        }
        return index - start;
    }
    if first.is_ascii_digit() {
        let mut index = start;
        while index < chars.len()
            && (chars[index].is_alphanumeric() || chars[index] == '_' || chars[index] == '.')
        {
            index += 1;
        }
        return index - start;
    }
    if is_operator(first) {
        let mut index = start;
        while index < chars.len() && is_operator(chars[index]) {
            index += 1;
        }
        return index - start;
    }
    1
}

fn is_operator(character: char) -> bool {
    matches!(
        character,
        '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | ':'
    )
}

fn underline_padding(line: &str, column: usize) -> String {
    line.chars()
        .take(column.saturating_sub(1))
        .map(|character| if character == '\t' { '\t' } else { ' ' })
        .collect()
}

fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", character as u32);
            }
            character => out.push(character),
        }
    }
    out
}

/// Suggest the closest candidate name using Levenshtein edit distance.
///
/// Only candidates that start with the same character (or are one edit away)
/// are considered, which avoids surprising suggestions for short names.
pub fn suggest_name<'a>(
    name: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    let target = name.to_lowercase();
    if target.is_empty() {
        return None;
    }
    let target_first = target.chars().next();
    let limit = (target.chars().count() / 3).max(1) + 1;
    let mut best: Option<(usize, &'a str)> = None;
    for candidate in candidates {
        if candidate.is_empty() {
            continue;
        }
        let lower = candidate.to_lowercase();
        if lower == target {
            continue;
        }
        let distance = edit_distance(&target, &lower);
        let same_first = lower.chars().next() == target_first;
        if distance > limit && !(same_first && distance <= limit + 1) {
            continue;
        }
        if !same_first && distance > 1 {
            continue;
        }
        if best.is_none_or(|(best_distance, _)| distance < best_distance) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// Optimal-string-alignment distance: insertion, deletion, substitution and
/// swapping two adjacent characters each cost one.
fn edit_distance(left: &str, right: &str) -> usize {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    let mut older: Vec<usize> = Vec::new();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_character) in left.iter().enumerate() {
        let index = row + 1;
        let mut current = vec![index; right.len() + 1];
        for (column, right_character) in right.iter().enumerate() {
            let offset = column + 1;
            let substitution = previous[column] + usize::from(left_character != right_character);
            current[offset] = substitution
                .min(current[column] + 1)
                .min(previous[offset] + 1);
            if index > 1
                && column > 0
                && left_character == &right[column - 1]
                && left[index - 2] == *right_character
            {
                current[offset] = current[offset].min(older[column - 1] + 1);
            }
        }
        older = previous;
        previous = current;
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "\
SAMPLE: [i32] <- [i32: 1, 2, 3]

flow reader => i32 {
    mut i: u64 <- 0
    // fixture line
    // fixture line
    // fixture line

    mut s <- give(SAMPLE)
    // fixture line
    // fixture line
    // fixture line
    // fixture line
    => SAMPLE[i]
}
";

    fn moved_diagnostic() -> Diagnostic {
        Diagnostic::from_error("use of moved value 'SAMPLE'", Span::new(14, 8))
            .with_label("used after move")
            .with_secondary(Span::new(9, 19), "moved here")
            .with_help("use clone(SAMPLE) to keep the original")
    }

    #[test]
    fn renders_the_target_layout_without_colors() {
        let rendered = render_one(&moved_diagnostic(), SOURCE, "file.loi", false);
        let expected = "\
error: use of moved value 'SAMPLE'
  --> file.loi:14:8
   |
 9 |     mut s <- give(SAMPLE)
   |                   ------ moved here
14 |     => SAMPLE[i]
   |        ^^^^^^ used after move
   |
   = help: use clone(SAMPLE) to keep the original
";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn discovers_identifier_and_string_extents() {
        assert_eq!(token_length("    => SAMPLE[i]", 8), 6);
        assert_eq!(token_length("    mut name <- give(NAME)", 9), 4);
        assert_eq!(token_length("    mut s <- give(SAMPLE)", 9), 1);
        assert_eq!(token_length("    mut s <- give(SAMPLE)", 19), 6);
        assert_eq!(token_length("NAME: [u8] <- \"hi\"", 15), 4);
        assert_eq!(token_length("x <- \"hello\"", 6), 7);
        assert_eq!(token_length("a < b", 3), 1);
        assert_eq!(token_length("a <- b", 3), 2);
    }

    #[test]
    fn prefers_explicit_span_ends() {
        let span = Span::new(1, 2).with_end_column(5);
        assert_eq!(span.end_line(), 1);
        assert_eq!(label_extent("abcdef", span), 3);

        let lexeme = Span::new(1, 1).with_lexeme("héllo");
        assert_eq!(lexeme.end_line, Some(1));
        assert_eq!(lexeme.end_column, Some(6));
        assert_eq!(label_extent("héllo rest", lexeme), 5);
    }

    #[test]
    fn multi_line_spans_underline_the_first_line_and_ellipsis() {
        let source = "flow main {\n    xs <- [1,\n           2]\n}\n";
        let diagnostic = Diagnostic::from_error("bad slice", Span::new(2, 11).with_end(3, 14));
        let rendered = render_one(&diagnostic, source, "file.loi", false);
        assert!(
            rendered.contains("2 |     xs <- [1,\n  |           ^^^...\n"),
            "unexpected rendering: {rendered}"
        );

        let json = render_all(&[diagnostic], source, "file.loi", ErrorFormat::Json, false);
        assert!(json.contains("\"line\":2,\"column\":11,\"end_line\":3,\"end_column\":14"));
    }

    #[test]
    fn underlines_keep_tabs_and_utf8_columns() {
        let source = "\t=> héllo + 1\n";
        let span = Span::new(1, 5).with_end(1, 10);
        let rendered = render_one(
            &Diagnostic::from_error("bad name", span),
            source,
            "file.loi",
            false,
        );
        assert!(
            rendered.contains("\t=> héllo + 1") && rendered.contains("\t   ^^^^^"),
            "unexpected rendering: {rendered}"
        );
    }

    #[test]
    fn short_format_keeps_the_historical_line() {
        let rendered = render_all(
            &[moved_diagnostic()],
            SOURCE,
            "file.loi",
            ErrorFormat::Short,
            false,
        );
        assert_eq!(rendered, "file.loi:14:8: use of moved value 'SAMPLE'\n");
    }

    #[test]
    fn json_format_escapes_text_and_includes_labels() {
        let diagnostic = Diagnostic::from_error("bad \"name\"\nhere", Span::new(2, 3))
            .with_secondary(Span::new(1, 1), "declared here");
        let rendered = render_all(&[diagnostic], "", "a.loi", ErrorFormat::Json, false);
        assert!(rendered.starts_with('['));
        assert!(rendered.contains("\\\"name\\\""));
        assert!(rendered.contains("\\n"));
        assert!(rendered.contains("\"end_line\":2") && rendered.contains("\"end_column\":3"));
        assert!(rendered.contains("\"secondary\":[{\"file\":\"a.loi\",\"line\":1,\"column\":1,\"end_line\":1,\"end_column\":1,\"label\":\"declared here\"}]"));
        assert!(rendered.trim_end().ends_with(']'));
    }

    #[test]
    fn sorts_and_summarizes_errors() {
        let later = Diagnostic::from_error("later", Span::new(9, 2));
        let earlier = Diagnostic::from_error("earlier", Span::new(2, 5));
        let rendered = render_all(
            &[later, earlier],
            "a\nb\nc\nd\ne\nf\ng\nh\ni\n",
            "file.loi",
            ErrorFormat::Human,
            false,
        );
        let earlier_index = rendered.find("error: earlier").expect("earlier header");
        let later_index = rendered.find("error: later").expect("later header");
        assert!(earlier_index < later_index);
        assert!(rendered.trim_end().ends_with("2 errors"));
    }

    #[test]
    fn suggests_close_names() {
        assert_eq!(suggest_name("SAMPEL", ["SAMPLE", "OTHER"]), Some("SAMPLE"));
        assert_eq!(suggest_name("name", ["name"]), None);
        assert_eq!(suggest_name("zzzz", ["SAMPLE"]), None);
        assert_eq!(edit_distance("cuonter", "counter"), 1);
        assert_eq!(suggest_name("cuonter", ["counter"]), Some("counter"));
    }

    #[test]
    fn colors_are_plain_ansi_when_enabled() {
        let rendered = render_one(&moved_diagnostic(), SOURCE, "file.loi", true);
        assert!(rendered.contains(RED));
        assert!(rendered.contains(BLUE));
        assert!(rendered.contains(CYAN));
        assert!(rendered.contains(RESET));
    }
}
