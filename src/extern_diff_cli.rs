//! File-based entry point for the library surface comparison.
use std::collections::BTreeMap;
use std::path::Path;

use clap::{Args, ValueEnum};
use parenox::{ExternDiff, ExternSnapshot, Source, compare_externs, output};

use crate::config::ProjectConfig;
use crate::workspace::{DiskProject, FileSnapshot};

#[derive(Args)]
pub(crate) struct DiffOptions {
    /// Previous extern file; repeat to load multiple files.
    #[arg(long, required_unless_present = "before_config", value_name = "FILE")]
    before: Vec<String>,
    /// Current extern file; repeat to load multiple files.
    #[arg(long, required_unless_present = "after_config", value_name = "FILE")]
    after: Vec<String>,
    /// Previous release configuration (externs and source specs).
    #[arg(long, value_name = "FILE")]
    before_config: Option<String>,
    /// Current release configuration (externs and source specs).
    #[arg(long, value_name = "FILE")]
    after_config: Option<String>,
    /// Report format.
    #[arg(long, value_enum, default_value_t = DiffFormat::Plain)]
    format: DiffFormat,
    /// Shorthand for --format json.
    #[arg(long, conflicts_with = "format")]
    json: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum DiffFormat {
    Plain,
    Json,
}

struct Inputs {
    config: Option<String>,
    externs: Vec<String>,
}

struct Files {
    specs: BTreeMap<String, String>,
    externs: Vec<FileSnapshot>,
}

impl Files {
    fn spec_sources(&self) -> Vec<Source<'_>> {
        self.specs
            .iter()
            .map(|(path, source)| Source { path, source })
            .collect()
    }
}

impl Inputs {
    fn read(self, side: &str) -> Result<Files, String> {
        let mut config = if let Some(path) = self.config {
            ProjectConfig::read(Path::new(&path))?
        } else {
            ProjectConfig::default()
        };
        if !self.externs.is_empty() {
            config.externs = self.externs;
        }
        if config.externs.is_empty() {
            return Err(format!(
                "diff-externs requires --{side} or --{side}-config with externs"
            ));
        }
        let uses_source_specs = !config.paths.is_empty();
        config.dependency_externs.clear();
        let project = DiskProject::read(config)?;
        if uses_source_specs {
            project.require_source_files()?;
        }
        Ok(Files {
            specs: project.files,
            externs: project.externs,
        })
    }
}

fn compare(before: &Files, after: &Files) -> ExternDiff {
    let old_specs = before.spec_sources();
    let new_specs = after.spec_sources();
    let old_externs: Vec<_> = before.externs.iter().map(FileSnapshot::as_source).collect();
    let new_externs: Vec<_> = after.externs.iter().map(FileSnapshot::as_source).collect();
    compare_externs(
        ExternSnapshot {
            sources: &old_specs,
            externs: &old_externs,
        },
        ExternSnapshot {
            sources: &new_specs,
            externs: &new_externs,
        },
    )
}

pub(super) fn run(options: DiffOptions) -> Result<u8, String> {
    let before = Inputs {
        config: options.before_config,
        externs: options.before,
    };
    let after = Inputs {
        config: options.after_config,
        externs: options.after,
    };
    let report = compare(&before.read("before")?, &after.read("after")?);
    if options.json || matches!(options.format, DiffFormat::Json) {
        println!(
            "{}",
            serde_json::to_string(&report).map_err(|err| err.to_string())?
        );
    } else {
        print_plain(&report);
    }
    Ok(u8::from(!report.compatible))
}

fn print_plain(report: &ExternDiff) {
    for (side, snapshot) in [("before", &report.before), ("after", &report.after)] {
        for file in &snapshot.files {
            if !file.ok || !file.complete {
                println!("{side}:");
                print!("{}", output::plain(file));
            }
        }
    }
    for change in &report.changes {
        print!("{} {}", change.kind.label(), change.name);
        if let Some(count) = change.argument_count {
            print!(" at {count} argument{}", if count == 1 { "" } else { "s" });
        }
        println!();
        if let Some(detail) = &change.detail {
            println!("  {detail}");
        }
        for (side, definition) in [("before", &change.before), ("after", &change.after)] {
            if let Some(definition) = definition {
                println!(
                    "  {side} ({}:{}:{}): {}",
                    definition.origin.path.as_deref().unwrap_or("extern"),
                    definition.span.start.line,
                    definition.span.start.column,
                    definition.declaration
                );
            }
        }
    }
    let status = if report.compatible {
        "compatible"
    } else if report.complete {
        "breaking"
    } else {
        "incomplete"
    };
    println!("Extern diff: {status} ({} changes)", report.changes.len());
}
