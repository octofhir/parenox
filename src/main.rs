use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use parenox::{Session, Source, output};
use serde::Deserialize;
use serde_json::{Value, json};

const HELP: &str = "Parenox — experimental source-only Clojure checker

Usage:
  parenox check <file.clj>... [--format pretty|plain|json] [--color auto|always|never] [--externs file.edn]
  parenox serve
  parenox bench <file.clj> [--iterations N]

serve reads one JSON object per line: {\"id\":1,\"path\":\"demo.clj\",\"source\":\"...\"}.
Project requests: {\"id\":1,\"files\":[{\"path\":\"a.clj\",\"source\":\"...\"}],\"externs\":{\"path\":\"externs.edn\",\"source\":\"...\"}}.
Directories are checked recursively as one project; externs contain declared library contracts.
check exits 0 on success, 1 on diagnostics, 2 on I/O or usage errors.
Default format: pretty (stderr). plain and json use stdout; --json aliases --format json.
Color defaults to auto; NO_COLOR and TERM=dumb disable automatic color.
Positions use 1-based Unicode scalar columns and 0-based UTF-8 byte offsets.
";

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
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") => check(&args[1..]),
        Some("serve") if args.len() == 1 => serve(),
        Some("bench") => bench(&args[1..]),
        Some("--help" | "-h") | None => {
            print!("{HELP}");
            Ok(0)
        }
        _ => Err(HELP.into()),
    }
}

fn check(args: &[String]) -> Result<u8, String> {
    let options = CheckOptions::parse(args)?;
    if options.paths.is_empty() {
        return Err(HELP.into());
    }
    let files = read_sources(&options.paths)?;
    let externs = options
        .externs
        .as_ref()
        .map(|path| {
            fs::read_to_string(path)
                .map(|source| Snapshot {
                    path: path.clone(),
                    source,
                })
                .map_err(|error| format!("{path}: {error}"))
        })
        .transpose()?;
    if files.len() == 1 && externs.is_none() {
        let (path, source) = files.first_key_value().unwrap();
        let report = Session::default().check(path, source);
        match options.format {
            Format::Json => println!(
                "{}",
                serde_json::to_string(&report).map_err(|e| e.to_string())?
            ),
            Format::Pretty => eprint!(
                "{}",
                output::pretty(&report, source, options.color.enabled())
            ),
            Format::Plain => print!("{}", output::plain(&report)),
        }
        return Ok(u8::from(!report.ok));
    }
    let sources: Vec<_> = files
        .iter()
        .map(|(path, source)| Source { path, source })
        .collect();
    let result =
        Session::default().check_project(&sources, externs.as_ref().map(Snapshot::as_source));
    if options.format == Format::Json {
        println!(
            "{}",
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        );
    } else {
        for report in &result.files {
            let source = files
                .get(&report.path)
                .map(String::as_str)
                .or_else(|| {
                    externs
                        .as_ref()
                        .filter(|s| s.path == report.path)
                        .map(|s| s.source.as_str())
                })
                .unwrap();
            match options.format {
                Format::Pretty => eprint!(
                    "{}",
                    output::pretty(report, source, options.color.enabled())
                ),
                Format::Plain => print!("{}", output::plain(report)),
                Format::Json => unreachable!(),
            }
        }
        let summary = format!(
            "Project: {} source files, {} checked, {} reused, {:.3} ms",
            files.len(),
            result.stats.checked_functions,
            result.stats.reused_functions,
            result.stats.elapsed_us as f64 / 1000.0
        );
        if options.format == Format::Pretty {
            eprintln!("{summary}");
        } else {
            println!("{summary}");
        }
    }
    Ok(u8::from(!result.ok))
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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Format {
    #[default]
    Pretty,
    Plain,
    Json,
}

#[derive(Clone, Copy, Default)]
enum Color {
    #[default]
    Auto,
    Always,
    Never,
}

impl Color {
    fn enabled(self) -> bool {
        match self {
            Color::Always => true,
            Color::Never => false,
            Color::Auto => {
                io::stderr().is_terminal()
                    && env::var_os("NO_COLOR").is_none()
                    && env::var_os("TERM").is_none_or(|term| term != "dumb")
            }
        }
    }
}

#[derive(Default)]
struct CheckOptions {
    paths: Vec<String>,
    format: Format,
    color: Color,
    externs: Option<String>,
}

impl CheckOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut options = Self::default();
        let mut explicit_format = None;
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if arg == "--" {
                options.paths.extend(args.cloned());
                break;
            }
            if arg == "--externs" {
                options.externs =
                    Some(args.next().ok_or("--externs requires a file path")?.clone());
                continue;
            }
            let format_value = if arg == "--json" {
                Some("json")
            } else if arg == "--format" {
                Some(
                    args.next()
                        .ok_or("--format requires pretty, plain or json")?
                        .as_str(),
                )
            } else {
                arg.strip_prefix("--format=")
            };
            if let Some(value) = format_value {
                let format = match value {
                    "pretty" => Format::Pretty,
                    "plain" => Format::Plain,
                    "json" => Format::Json,
                    _ => {
                        return Err(format!(
                            "Unknown output format {value:?}; expected pretty, plain or json"
                        ));
                    }
                };
                if explicit_format.is_some_and(|previous| previous != format) {
                    return Err("Conflicting output formats; choose pretty, plain or json".into());
                }
                options.format = format;
                explicit_format = Some(format);
                continue;
            }
            let color_value = if arg == "--color" {
                Some(
                    args.next()
                        .ok_or("--color requires auto, always or never")?
                        .as_str(),
                )
            } else {
                arg.strip_prefix("--color=")
            };
            if let Some(value) = color_value {
                options.color = match value {
                    "auto" => Color::Auto,
                    "always" => Color::Always,
                    "never" => Color::Never,
                    _ => {
                        return Err(format!(
                            "Unknown color mode {value:?}; expected auto, always or never"
                        ));
                    }
                };
            } else if arg.starts_with('-') {
                return Err(format!("Unknown option {arg:?}\n\n{HELP}"));
            } else {
                options.paths.push(arg.clone());
            }
        }
        Ok(options)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRequest {
    #[serde(default)]
    id: Value,
    path: String,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    path: String,
    source: String,
}

impl Snapshot {
    fn as_source(&self) -> Source<'_> {
        Source {
            path: &self.path,
            source: &self.source,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectRequest {
    #[serde(default)]
    id: Value,
    files: Vec<Snapshot>,
    externs: Option<Snapshot>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Request {
    File(FileRequest),
    Project(ProjectRequest),
}

fn serve() -> Result<u8, String> {
    let mut session = Session::default();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(Request::File(request)) => {
                let report = session.check(&request.path, &request.source);
                json!({"id": request.id, "result": report})
            }
            Ok(Request::Project(request)) => {
                let sources: Vec<_> = request.files.iter().map(Snapshot::as_source).collect();
                let report = session
                    .check_project(&sources, request.externs.as_ref().map(Snapshot::as_source));
                json!({"id": request.id, "result": report})
            }
            Err(error) => {
                json!({"id": null, "error": {"code": "invalid_request", "message": error.to_string()}})
            }
        };
        serde_json::to_writer(&mut output, &response).map_err(|e| e.to_string())?;
        writeln!(output).map_err(|e| e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
    }
    Ok(0)
}

fn bench(args: &[String]) -> Result<u8, String> {
    let (path, iterations) = match args {
        [path] => (path, 1000),
        [path, flag, count] if flag == "--iterations" => (
            path,
            count
                .parse::<usize>()
                .map_err(|_| "Invalid iteration count")?,
        ),
        _ => return Err(HELP.into()),
    };
    if iterations == 0 || iterations > 1_000_000 {
        return Err("Iterations must be between 1 and 1000000".into());
    }
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
            "scope": "in-process, unchanged single-file source; includes parsing and model building; excludes file I/O, process startup and JSON transport"
        })
    );
    Ok(0)
}
