mod cli;
mod config;
mod extern_diff_cli;
mod serve;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use clap::{CommandFactory, Parser};
use parenox::{Diagnostic, Session, Source, output, reader};
use serde_json::json;

use cli::{CheckOptions, Cli, Color, Command, DefinitionOptions, Format, ProjectOptions};
use config::ProjectConfig;
use serve::Snapshot;

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
        Some(Command::Check(options)) => check(options, false),
        Some(Command::Definition(options)) => definition(options),
        Some(Command::ValidateExterns(options)) => check(options, true),
        Some(Command::ExportExterns(options)) => export_externs(options),
        Some(Command::DiffExterns(options)) => extern_diff_cli::run(options),
        Some(Command::Serve) => serve::run(),
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
    let files = read_sources(&config.paths)?;
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

fn check(mut options: CheckOptions, validate_externs: bool) -> Result<u8, String> {
    options.load_config()?;
    if validate_externs && options.externs.is_empty() {
        return Err("validate-externs requires at least one --externs file".into());
    }
    let files = read_sources(&options.project.paths)?;
    let format = options.format();
    let color = format == Format::Pretty && options.color.enabled();
    let externs = read_externs(&options.externs)?;
    let dependencies = read_externs(&options.dependency_externs)?;
    if files.len() == 1 && externs.is_empty() && dependencies.is_empty() {
        let (path, source) = files.first_key_value().unwrap();
        if format == Format::Pretty {
            eprint!("{}", output::checking(&format!("`{path}`"), color));
        }
        let report = Session::default().check(path, source);
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
        .map(Snapshot::as_source)
        .collect();
    let mut session = Session::default();
    let result = if validate_externs {
        let (checked, trusted) = extern_sources.split_at(externs.len());
        session.validate_externs_with_dependencies(&sources, checked, trusted)
    } else {
        session.check_project_with_externs(&sources, &extern_sources)
    };
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
                "Project: {} source files, {} checked, {} reused, {:.3} ms",
                files.len(),
                result.stats.checked_functions,
                result.stats.reused_functions,
                result.stats.elapsed_us as f64 / 1000.0
            );
        }
    }
    Ok(u8::from(
        !result.ok || (validate_externs && !result.complete),
    ))
}

fn export_externs(mut options: CheckOptions) -> Result<u8, String> {
    options.load_config()?;
    let files = read_sources(&options.project.paths)?;
    let sources: Vec<_> = files
        .iter()
        .map(|(path, source)| Source { path, source })
        .collect();
    options.externs.append(&mut options.dependency_externs);
    let dependencies = read_externs(&options.externs)?;
    let externs: Vec<_> = dependencies.iter().map(Snapshot::as_source).collect();
    let export = Session::default().export_externs(&sources, &externs);
    if let Some(edn) = export.edn {
        print!("{edn}");
        return Ok(0);
    }

    let format = options.format();
    let color = format == Format::Pretty && options.color.enabled();
    match format {
        Format::Json => eprintln!(
            "{}",
            serde_json::to_string(&export.report).map_err(|err| err.to_string())?
        ),
        Format::Plain | Format::Pretty => {
            for (file, source) in export
                .report
                .files
                .iter()
                .zip(sources.iter().chain(&externs))
            {
                match format {
                    Format::Pretty => {
                        eprint!("{}", output::file_diagnostics(file, source.source, color))
                    }
                    Format::Plain => eprint!("{}", output::plain(file)),
                    Format::Json => unreachable!(),
                }
            }
            eprintln!("No externs exported: the check did not pass completely.");
        }
    }
    Ok(1)
}

fn read_externs(paths: &[String]) -> Result<Vec<Snapshot>, String> {
    paths
        .iter()
        .map(|path| {
            fs::read_to_string(path)
                .map(|source| Snapshot {
                    path: path.clone(),
                    source,
                })
                .map_err(|error| format!("{path}: {error}"))
        })
        .collect()
}

fn read_sources(paths: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut files = BTreeMap::new();
    let mut pending: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    while let Some(path) = pending.pop() {
        let metadata =
            fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            for child in fs::read_dir(&path).map_err(|e| e.to_string())? {
                let child = child.map_err(|e| e.to_string())?.path();
                if child
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                {
                    continue;
                }
                pending.push(child);
            }
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("clj" | "cljc")
        ) {
            let source =
                fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            files.insert(path.to_string_lossy().into_owned(), source);
        }
    }
    if files.is_empty() {
        return Err("No Clojure source files found".into());
    }
    Ok(files)
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
    fn load_config(&mut self) -> Result<(), String> {
        let config = self.project.load()?;
        self.project.paths = config.paths;
        if self.externs.is_empty() {
            self.externs = config.externs;
        }
        if self.dependency_externs.is_empty() {
            self.dependency_externs = config.dependency_externs;
        }
        Ok(())
    }
}

impl ProjectOptions {
    fn load(&self) -> Result<ProjectConfig, String> {
        let default = Path::new(config::DEFAULT_PATH);
        let mut settings = if let Some(path) = &self.config {
            ProjectConfig::read(Path::new(path))?
        } else if self.paths.is_empty() && file_exists(default)? {
            ProjectConfig::read(default)?
        } else {
            ProjectConfig::default()
        };
        if !self.paths.is_empty() {
            settings.paths.clone_from(&self.paths);
        } else {
            let manifest = self.deps.as_deref().map(PathBuf::from).unwrap_or_else(|| {
                self.config
                    .as_deref()
                    .and_then(|path| Path::new(path).parent())
                    .unwrap_or_else(|| Path::new(""))
                    .join("deps.edn")
            });
            if self.deps.is_some()
                || !self.aliases.is_empty()
                || (settings.paths.is_empty() && file_exists(&manifest)?)
            {
                settings.paths = config::deps_paths(&manifest, &self.aliases)?;
            }
        }
        if settings.paths.is_empty() {
            return Err(
                "No source paths. Specify paths or configure :paths in parenox.edn or deps.edn"
                    .into(),
            );
        }
        Ok(settings)
    }
}

fn file_exists(path: &Path) -> Result<bool, String> {
    path.try_exists()
        .map_err(|err| format!("{}: {err}", path.display()))
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
