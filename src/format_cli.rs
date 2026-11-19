//! CLI file selection and I/O for the shared formatter.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use parenox::formatter::{self, Language};

use crate::cli::FmtOptions;
use crate::workspace::matching_paths;

pub(super) fn run(options: FmtOptions) -> Result<u8, String> {
    let settings = options.project.load_for_format(Path::new(""))?;
    let files = matching_paths(&settings.paths, |path| Language::for_path(path).is_some())?;
    if files.is_empty() {
        return Err("No Clojure or EDN files found".into());
    }
    if !options.check && !options.write && files.len() != 1 {
        return Err("Use --check or --write to format multiple files".into());
    }
    let mut changed = false;
    let mut invalid = false;
    let mut writes = Vec::new();
    for path in files {
        let label = path.display();
        let source = fs::read_to_string(&path).map_err(|error| format!("{label}: {error}"))?;
        let formatted = match formatter::format(
            &source,
            Language::for_path(&path).unwrap(),
            &settings.format,
        ) {
            Ok(formatted) => formatted,
            Err(error) => {
                eprintln!(
                    "{label}:{}:{}: {}",
                    error.span.start.line, error.span.start.column, error.message
                );
                invalid = true;
                continue;
            }
        };
        if !options.check && !options.write {
            io::stdout()
                .lock()
                .write_all(formatted.as_bytes())
                .map_err(|error| error.to_string())?;
        } else if formatted != source {
            changed = true;
            if options.check {
                eprintln!("{label}: would reformat");
            } else {
                writes.push((path, formatted));
            }
        }
    }
    // Don't rewrite valid files in a batch that contains invalid source.
    if invalid {
        return Ok(1);
    }
    for (path, text) in writes {
        fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(u8::from(options.check && changed))
}
