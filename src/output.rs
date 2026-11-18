//! Human-readable diagnostics. JSON is serialized directly from `Report` by the CLI.

use std::fmt::Write;

use annotate_snippets::{AnnotationKind, Group, Level, Patch, Renderer, Snippet};

use annotate_snippets::renderer::{AnsiColor, Effects, Style};

use crate::{ContractKind, ContractOrigin, Diagnostic, ProjectReport, Report, Stats};

pub fn contract_note(origin: &ContractOrigin) -> String {
    let kind = match origin.kind {
        ContractKind::TSig => ":t/sig",
        ContractKind::SFdef => "s/fdef",
        ContractKind::Extern => "extern declaration",
        ContractKind::ExternLiteral => "extern literal value",
        ContractKind::Builtin => "built-in rule",
        ContractKind::MacroScope => "macro scope declaration",
    };
    match &origin.path {
        Some(path) => format!("Contract for {}: {kind} in {path}", origin.name),
        None => format!("Contract for {}: {kind}", origin.name),
    }
}

fn renderer(color: bool) -> Renderer {
    if color {
        Renderer::styled()
    } else {
        Renderer::plain()
    }
}

pub fn diagnostics(path: &str, source: &str, diagnostics: &[Diagnostic], color: bool) -> String {
    let renderer = renderer(color);
    let mut output = String::new();
    for diagnostic in diagnostics {
        let title = diagnostic
            .headline
            .as_deref()
            .unwrap_or(&diagnostic.message);
        let mut annotation =
            AnnotationKind::Primary.span(diagnostic.span.start.offset..diagnostic.span.end.offset);
        if let (Some(expected), Some(actual)) = (&diagnostic.expected, &diagnostic.actual) {
            annotation = annotation.label(format!("expected `{expected}`, found `{actual}`"));
        }
        let mut primary = Level::ERROR
            .primary_title(title)
            .id(&diagnostic.code)
            .element(
                Snippet::source(source)
                    .path(path)
                    .fold(true)
                    .annotation(annotation),
            );
        if let Some(contract) = &diagnostic.contract {
            primary = primary.element(Level::NOTE.message(contract_note(contract)));
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
                        .path(path)
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
    output
}

pub fn file_diagnostics(report: &Report, source: &str, color: bool) -> String {
    let mut output = diagnostics(&report.path, source, &report.diagnostics, color);
    if !report.suppressed.is_empty() {
        let renderer = renderer(color);
        for finding in &report.suppressed {
            let message = suppression_note(&report.path, finding);
            let note = Group::with_title(Level::NOTE.secondary_title(message));
            writeln!(output, "{}", renderer.render(&[note])).unwrap();
        }
    }
    output
}

fn suppression_note(path: &str, finding: &crate::SuppressedDiagnostic) -> String {
    let start = finding.diagnostic.span.start;
    format!(
        "suppressed {} at {path}:{}:{}: {}",
        finding.diagnostic.code, start.line, start.column, finding.reason
    )
}

fn status(label: &str, message: std::fmt::Arguments<'_>, color: bool) -> String {
    let style = if color {
        AnsiColor::BrightGreen.on_default().effects(Effects::BOLD)
    } else {
        Style::new()
    };
    format!("{style}{label:>12}{style:#} {message}\n")
}

pub fn checking(target: &str, color: bool) -> String {
    status("Checking", format_args!("{target}"), color)
}

fn summary(
    target: &str,
    count: usize,
    suppressed: usize,
    complete: bool,
    stats: &Stats,
    color: bool,
) -> String {
    if count == 0 {
        let result = if suppressed == 0 {
            "no type errors".into()
        } else {
            format!("no unsuppressed type errors; {suppressed} suppressed")
        };
        return status(
            "Finished",
            format_args!(
                "checking {target} — {result} ({} checked, {} reused, {:.3} ms)",
                stats.checked_functions,
                stats.reused_functions,
                stats.elapsed_us as f64 / 1000.0
            ),
            color,
        );
    }

    let renderer = renderer(color);
    let mut output = String::new();
    if !complete {
        let message = if suppressed == 0 {
            "check is incomplete; some constructs could not be analyzed"
        } else {
            "check is incomplete; findings were suppressed or analysis was limited"
        };
        let note = Group::with_title(Level::NOTE.secondary_title(message));
        writeln!(output, "{}", renderer.render(&[note])).unwrap();
    }
    let message = format!(
        "could not check {target} due to {count} previous error{}",
        if count == 1 { "" } else { "s" }
    );
    writeln!(
        output,
        "{}",
        renderer.render(&[Group::with_title(Level::ERROR.primary_title(&message))])
    )
    .unwrap();
    output
}

pub fn pretty(report: &Report, source: &str, color: bool) -> String {
    let mut output = file_diagnostics(report, source, color);
    output.push_str(&summary(
        &format!("`{}`", report.path),
        report.diagnostics.len(),
        report.suppressed.len(),
        report.complete,
        &report.stats,
        color,
    ));
    output
}

pub fn project_summary(report: &ProjectReport, target: &str, color: bool) -> String {
    summary(
        target,
        report.files.iter().map(|file| file.diagnostics.len()).sum(),
        report.files.iter().map(|file| file.suppressed.len()).sum(),
        report.complete,
        &report.stats,
        color,
    )
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
        if let Some(contract) = &diagnostic.contract {
            writeln!(output, "  note: {}", contract_note(contract)).unwrap();
        }
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
    for finding in &report.suppressed {
        writeln!(
            output,
            "  note: {}",
            suppression_note(&report.path, finding)
        )
        .unwrap();
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
