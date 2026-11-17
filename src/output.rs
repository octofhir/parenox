//! Human-readable diagnostics. JSON is serialized directly from `Report` by the CLI.

use std::fmt::Write;

use annotate_snippets::{AnnotationKind, Group, Level, Patch, Renderer, Snippet};

use crate::Report;

pub fn pretty(report: &Report, source: &str, color: bool) -> String {
    let renderer = if color {
        Renderer::styled()
    } else {
        Renderer::plain()
    };
    let mut output = String::new();
    for diagnostic in &report.diagnostics {
        let label = match &diagnostic.actual {
            Some(actual) => format!("this expression has type `{actual}`"),
            None => diagnostic.message.clone(),
        };
        let mut primary = Level::ERROR
            .primary_title(&diagnostic.message)
            .id(&diagnostic.code)
            .element(
                Snippet::source(source)
                    .path(&report.path)
                    .fold(true)
                    .annotation(
                        AnnotationKind::Primary
                            .span(diagnostic.span.start.offset..diagnostic.span.end.offset)
                            .label(label),
                    ),
            );
        if let (Some(expected), Some(actual)) = (&diagnostic.expected, &diagnostic.actual) {
            primary = primary.element(
                Level::NOTE.message(format!("expected `{expected}`\n   found `{actual}`")),
            );
        }
        for note in &diagnostic.notes {
            primary = primary.element(Level::NOTE.message(note));
        }
        if let Some(help) = &diagnostic.help {
            primary = primary.element(Level::HELP.message(help));
        }
        let mut groups = vec![primary];
        for suggestion in &diagnostic.suggestions {
            groups.push(
                Group::with_title(Level::HELP.secondary_title(&suggestion.message)).element(
                    Snippet::source(source)
                        .path(&report.path)
                        .fold(true)
                        .patch(Patch::new(
                            suggestion.span.start.offset..suggestion.span.end.offset,
                            &suggestion.replacement,
                        )),
                ),
            );
        }
        writeln!(output, "{}\n", renderer.render(&groups)).unwrap();
    }
    if !report.complete {
        writeln!(
            output,
            "note: check is incomplete; some constructs could not be analyzed\n"
        )
        .unwrap();
    }
    if report.ok {
        let finished = if color {
            "\x1b[1;32m    Finished\x1b[0m"
        } else {
            "    Finished"
        };
        writeln!(
            output,
            "{finished} checking `{}` — no type errors",
            report.path
        )
        .unwrap();
    } else {
        let error = if color {
            "\x1b[1;31merror\x1b[0m"
        } else {
            "error"
        };
        let count = report.diagnostics.len();
        writeln!(
            output,
            "{error}: could not check `{}` due to {count} previous error{}",
            report.path,
            if count == 1 { "" } else { "s" }
        )
        .unwrap();
    }
    writeln!(
        output,
        "    {} checked, {} reused, {:.3} ms",
        report.stats.checked_functions,
        report.stats.reused_functions,
        report.stats.elapsed_us as f64 / 1000.0
    )
    .unwrap();
    output
}

pub fn plain(report: &Report) -> String {
    let mut output = String::new();
    for diagnostic in &report.diagnostics {
        writeln!(
            output,
            "{}:{}:{}: {} {}",
            report.path,
            diagnostic.span.start.line,
            diagnostic.span.start.column,
            diagnostic.code,
            diagnostic.message
        )
        .unwrap();
        for note in &diagnostic.notes {
            writeln!(output, "  note: {note}").unwrap();
        }
        if let Some(help) = &diagnostic.help {
            writeln!(output, "  help: {help}").unwrap();
        }
        for suggestion in &diagnostic.suggestions {
            writeln!(
                output,
                "  help: {}; replace {}:{}..{}:{} with {:?} (requires review)",
                suggestion.message,
                suggestion.span.start.line,
                suggestion.span.start.column,
                suggestion.span.end.line,
                suggestion.span.end.column,
                suggestion.replacement
            )
            .unwrap();
        }
    }
    writeln!(
        output,
        "{}: {} ({} checked, {} reused, {} us, complete={})",
        report.path,
        if report.ok { "OK" } else { "FAILED" },
        report.stats.checked_functions,
        report.stats.reused_functions,
        report.stats.elapsed_us,
        report.complete
    )
    .unwrap();
    output
}
