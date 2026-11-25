mod cli;
mod config;
mod extern_diff_cli;
mod format_cli;
mod lsp;
mod nrepl;
mod runtime;
mod serve;
mod workspace;

use std::env;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use clap::{CommandFactory, Parser};
use parenox::{Diagnostic, ProjectReport, Report, Session, Source, output, reader};
use serde_json::json;

use cli::{CheckOptions, Cli, Color, Command, DefinitionOptions, Format};
use config::ProjectConfig;
use workspace::{DiskProject, FileSnapshot};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("parenox: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<u8, String> {
    match Cli::parse().command {
        Some(Command::Check {
            options,
            allow_incomplete,
            check_unannotated,
        }) => check(
            options,
            if allow_incomplete {
                CheckMode::AllowIncomplete
            } else {
                CheckMode::Strict
            },
            check_unannotated,
        ),
        Some(Command::Eval(options)) => runtime::eval(options),
        Some(Command::Run(options)) => runtime::run(options),
        Some(Command::Test(options)) => runtime::run_tests(options),
        Some(Command::Nrepl(options)) => nrepl::run(options),
        Some(Command::Definition(options)) => definition(options),
        Some(Command::ValidateExterns(options)) => {
            check(options, CheckMode::ValidateExterns, false)
        }
        Some(Command::ExportExterns(options)) => export_externs(options),
        Some(Command::ProposeContracts {
            options,
            specs,
            externs_only,
        }) => propose_contracts(options, &specs, externs_only),
        Some(Command::Fmt(options)) => format_cli::run(options),
        Some(Command::DiffExterns(options)) => extern_diff_cli::run(options),
        Some(Command::Serve) => serve::run(),
        Some(Command::Lsp(options)) => lsp::run(options),
        Some(Command::Syntax { path }) => syntax(&path),
        Some(Command::Bench { path, iterations }) => bench(&path, iterations as usize),
        None => {
            Cli::command().print_help().map_err(|err| err.to_string())?;
            println!();
            Ok(0)
        }
    }
}

fn syntax(path: &str) -> Result<u8, String> {
    let source = fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    let parsed = reader::parse(&source);
    print!("{:#?}", parsed.syntax());
    let diagnostics: Vec<_> = parsed.errors.into_iter().map(Diagnostic::from).collect();
    eprint!(
        "{}",
        output::diagnostics(path, &source, &diagnostics, Color::Auto.enabled())
    );
    Ok(u8::from(!diagnostics.is_empty()))
}

fn definition(options: DefinitionOptions) -> Result<u8, String> {
    let config = options.project.load()?;
    let files = DiskProject::read_sources(&config)?;
    if files.is_empty() {
        return Err("No Clojure source files found".into());
    }
    let requested = Path::new(&options.file);
    let file = if files.contains_key(&options.file) {
        Some(options.file.as_str())
    } else {
        requested.canonicalize().ok().and_then(|requested| {
            files
                .keys()
                .find(|path| {
                    Path::new(path)
                        .canonicalize()
                        .is_ok_and(|found| found == requested)
                })
                .map(String::as_str)
        })
    };
    let target = file.and_then(|path| {
        let sources: Vec<_> = files
            .iter()
            .map(|(path, source)| Source { path, source })
            .collect();
        let mut session = Session::default();
        session.check_project(&sources, None);
        session.definition(path, options.offset)
    });
    println!(
        "{}",
        serde_json::to_string(&target).map_err(|err| err.to_string())?
    );
    Ok(u8::from(target.is_none()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CheckMode {
    Strict,
    AllowIncomplete,
    ValidateExterns,
}

impl CheckMode {
    fn apply(self, report: &mut Report) {
        if self == Self::AllowIncomplete {
            report.ok = report
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity == "warning");
        }
    }
}

fn check(options: CheckOptions, mode: CheckMode, check_unannotated: bool) -> Result<u8, String> {
    let validate_externs = mode == CheckMode::ValidateExterns;
    let config = options.resolved_config()?;
    if validate_externs && config.externs.is_empty() {
        return Err("validate-externs requires at least one --externs file".into());
    }
    let project = DiskProject::read(config)?;
    project.require_source_files()?;
    let DiskProject {
        files,
        externs,
        dependency_externs: dependencies,
    } = project;
    let format = options.format();
    let color = format == Format::Pretty && options.color.enabled();
    let mut session = Session::default().with_unannotated_bodies(check_unannotated);
    if files.len() == 1 && externs.is_empty() && dependencies.is_empty() {
        let (path, source) = files.first_key_value().unwrap();
        if format == Format::Pretty {
            eprint!("{}", output::checking(&format!("`{path}`"), color));
        }
        let mut report = session.check(path, source);
        mode.apply(&mut report);
        match format {
            Format::Json => println!(
                "{}",
                serde_json::to_string(&report).map_err(|e| e.to_string())?
            ),
            Format::Pretty => eprint!("{}", output::pretty(&report, source, color)),
            Format::Plain => print!("{}", output::plain(&report)),
        }
        return Ok(u8::from(!report.ok));
    }
    let sources: Vec<_> = files
        .iter()
        .map(|(path, source)| Source { path, source })
        .collect();
    let target = format!(
        "{} ({} source file{})",
        if validate_externs {
            "extern validation"
        } else {
            "project"
        },
        files.len(),
        if files.len() == 1 { "" } else { "s" }
    );
    if format == Format::Pretty {
        eprint!("{}", output::checking(&target, color));
    }
    let extern_sources: Vec<_> = externs
        .iter()
        .chain(&dependencies)
        .map(FileSnapshot::as_source)
        .collect();
    let mut result = if validate_externs {
        let (checked, trusted) = extern_sources.split_at(externs.len());
        session.validate_externs_with_dependencies(&sources, checked, trusted)
    } else {
        session.check_project_with_externs(&sources, &extern_sources)
    };
    if mode == CheckMode::AllowIncomplete {
        for report in &mut result.files {
            mode.apply(report);
        }
        result.ok = result.files.iter().all(|report| report.ok);
    }
    if format == Format::Json {
        println!(
            "{}",
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        );
    } else {
        for (report, source) in result
            .files
            .iter()
            .zip(sources.iter().chain(&extern_sources))
        {
            match format {
                Format::Pretty => {
                    eprint!("{}", output::file_diagnostics(report, source.source, color))
                }
                Format::Plain => print!("{}", output::plain(report)),
                Format::Json => unreachable!(),
            }
        }
        if format == Format::Pretty {
            eprint!("{}", output::project_summary(&result, &target, color));
        } else {
            println!(
                "Project: {} source files, {} checked, {} reused, {:.3} ms{}",
                files.len(),
                result.stats.checked_functions,
                result.stats.reused_functions,
                result.stats.elapsed_us as f64 / 1000.0,
                if result.complete {
                    ""
                } else {
                    " (analysis incomplete)"
                },
            );
        }
    }
    Ok(u8::from(
        !result.ok || (validate_externs && !result.complete),
    ))
}

fn export_externs(options: CheckOptions) -> Result<u8, String> {
    let project = DiskProject::read(options.resolved_config()?)?;
    project.require_source_files()?;
    let sources = project.sources();
    let externs = project.all_externs();
    let export = Session::default().export_externs(&sources, &externs);
    if let Some(edn) = export.edn {
        print!("{edn}");
        return Ok(0);
    }

    print_export_diagnostics(&options, &export.report, &sources, &externs)?;
    if options.format() != Format::Json {
        eprintln!("No externs exported: the check did not pass completely.");
    }
    Ok(1)
}

fn propose_contracts(
    options: CheckOptions,
    specs: &[String],
    externs_only: bool,
) -> Result<u8, String> {
    let project = DiskProject::read(options.resolved_config()?)?;
    project.require_source_files()?;
    let sources = project.sources();
    let externs = project.all_externs();
    let result = Session::default().propose_contracts(&sources, &externs, specs)?;
    let edn = if externs_only {
        result.externs
    } else {
        result.edn
    };
    if let Some(edn) = edn {
        print!("{edn}");
        return Ok(0);
    }
    print_export_diagnostics(&options, &result.report, &sources, &externs)?;
    Ok(1)
}

fn print_export_diagnostics(
    options: &CheckOptions,
    report: &ProjectReport,
    sources: &[Source<'_>],
    externs: &[Source<'_>],
) -> Result<(), String> {
    let format = options.format();
    let color = format == Format::Pretty && options.color.enabled();
    match format {
        Format::Json => eprintln!(
            "{}",
            serde_json::to_string(&report).map_err(|err| err.to_string())?
        ),
        Format::Plain | Format::Pretty => {
            for (file, source) in report.files.iter().zip(sources.iter().chain(externs)) {
                match format {
                    Format::Pretty => {
                        eprint!("{}", output::file_diagnostics(file, source.source, color))
                    }
                    Format::Plain => eprint!("{}", output::plain(file)),
                    Format::Json => unreachable!(),
                }
            }
        }
    }
    Ok(())
}

impl Color {
    fn enabled(self) -> bool {
        match self {
            Color::Always => true,
            Color::Never => false,
            Color::Auto => {
                env::var_os("NO_COLOR").is_none()
                    && (env::var_os("CLICOLOR_FORCE")
                        .is_some_and(|value| !value.is_empty() && value != "0")
                        || (io::stderr().is_terminal()
                            && env::var_os("TERM").is_none_or(|term| term != "dumb")))
            }
        }
    }
}

impl CheckOptions {
    fn resolved_config(&self) -> Result<ProjectConfig, String> {
        let mut config = self.project.load()?;
        if !self.externs.is_empty() {
            config.externs.clone_from(&self.externs);
        }
        if !self.dependency_externs.is_empty() {
            config
                .dependency_externs
                .clone_from(&self.dependency_externs);
        }
        Ok(config)
    }
}

fn bench(path: &str, iterations: usize) -> Result<u8, String> {
    let source = fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let mut session = Session::default();
    let started = Instant::now();
    let initial = session.check(path, &source);
    let first_us = started.elapsed().as_micros();
    if !initial.ok {
        eprint!(
            "{}",
            output::pretty(&initial, &source, Color::Auto.enabled())
        );
        return Ok(1);
    }
    let mut times = Vec::with_capacity(iterations);
    let mut reused = 0;
    for _ in 0..iterations {
        let started = Instant::now();
        let report = session.check(path, &source);
        times.push(started.elapsed().as_micros());
        reused += report.stats.reused_functions;
    }
    times.sort_unstable();
    println!(
        "{}",
        json!({
            "path": path,
            "iterations": iterations,
            "first_check_us": first_us,
            "warm_p50_us": times[(iterations - 1) / 2],
            "warm_p95_us": times[(iterations * 95).div_ceil(100) - 1],
            "warm_max_us": times[iterations - 1],
            "reused_functions_total": reused,
            "scope": "in-process, unchanged single-file source; warm checks reuse parsing, declaration models and function diagnostics; excludes file I/O, process startup and JSON transport"
        })
    );
    Ok(0)
}
