//! Public diagnostic rendering contract: severity counts must reflect the
//! diagnostics' own severity, not the number of items in the slice.

use loi::diagnostic::{SourcedDiagnostic, render_sourced};
use loi::{Diagnostic, ErrorFormat, Span};

fn item(diagnostic: Diagnostic) -> SourcedDiagnostic {
    SourcedDiagnostic {
        diagnostic,
        source: "mut x: i32 <- 0".to_string(),
        file: "main.loi".to_string(),
    }
}

#[test]
fn sourced_rendering_summarizes_warnings_as_warnings() {
    let only_warning = [item(Diagnostic::warning(
        "unused variable 'x'",
        Span::new(1, 1),
    ))];
    let rendered = render_sourced(&only_warning, ErrorFormat::Human, false);
    assert!(
        rendered.ends_with("1 warning\n"),
        "unexpected summary: {rendered}"
    );

    let mixed = [
        item(Diagnostic::error("unknown name", Span::new(1, 1))),
        item(Diagnostic::warning("unused variable 'x'", Span::new(1, 1))),
    ];
    let rendered = render_sourced(&mixed, ErrorFormat::Human, false);
    assert!(
        rendered.ends_with("1 error, 1 warning\n"),
        "unexpected summary: {rendered}"
    );
}
