//! Explicit Clojure CLI invocations. Static analysis remains source-only.
use std::env;
use std::fs::{self, DirBuilder};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

use parenox::reader::{self, Form};

use crate::cli::TestOptions;

const RUNNER: &str = include_str!("runtime/clojure_test.clj");
const RUNNER_ALIAS: &str = "parenox/test-runner";
const RUNNER_DEPS: &str = "{:aliases {:parenox/test-runner {:main-opts []}}}";

pub(super) fn run_tests(options: TestOptions) -> Result<u8, String> {
    let project = options
        .project
        .canonicalize()
        .map_err(|error| format!("{}: {error}", options.project.display()))?;
    if !project.join("deps.edn").is_file() {
        return Err(format!(
            "{}: expected a deps.edn project",
            project.display()
        ));
    }
    let aliases = cli_aliases(&options.aliases)?;
    for name in &options.namespaces {
        if name.contains(['/', ':'])
            || !single_form(name).is_some_and(|form| form.symbol() == Some(name))
        {
            return Err(format!("{name:?} is not a Clojure namespace name"));
        }
    }

    let executable =
        if options.clojure.is_relative() && options.clojure.parent() != Some(Path::new("")) {
            env::current_dir()
                .map_err(|error| error.to_string())?
                .join(options.clojure)
        } else {
            options.clojure
        };
    let run =
        RunDirectory::create().map_err(|error| format!("create test run directory: {error}"))?;
    let script = run.0.join("clojure_test.clj");
    let result = run.0.join("result");
    fs::write(&script, RUNNER).map_err(|error| format!("write test runner: {error}"))?;
    let status = Command::new(&executable)
        .current_dir(&project)
        .args(["-Sdeps", RUNNER_DEPS, &aliases])
        .arg(script)
        .arg(&result)
        .args(options.namespaces)
        .status()
        .map_err(|error| format!("could not start {}: {error}", executable.display()))?;
    if !status.success() {
        return Err(format!("Clojure test run did not complete ({status})"));
    }
    match fs::read_to_string(result).as_deref() {
        Ok("pass") => Ok(0),
        Ok("fail") => Ok(1),
        Ok("empty") => Err("no clojure.test vars ran in the requested namespaces".into()),
        _ => Err("Clojure exited without a test result".into()),
    }
}

fn cli_aliases(aliases: &[String]) -> Result<String, String> {
    let mut combined = String::from("-M");
    for alias in aliases {
        let name = alias.strip_prefix(':').unwrap_or(alias);
        let keyword = format!(":{name}");
        if name.is_empty()
            || name.contains(':')
            || name == RUNNER_ALIAS
            || !single_form(&keyword).is_some_and(|form| form.keyword() == Some(&keyword))
        {
            return Err(format!("{alias:?} is not a valid test alias"));
        }
        combined.push_str(&keyword);
    }
    combined.push(':');
    combined.push_str(RUNNER_ALIAS);
    Ok(combined)
}

fn single_form(source: &str) -> Option<Form> {
    let mut forms = reader::read(source).ok()?;
    (forms.len() == 1).then(|| forms.remove(0))
}

struct RunDirectory(PathBuf);

impl RunDirectory {
    fn create() -> io::Result<Self> {
        let root = env::temp_dir().canonicalize()?;
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut builder = DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        for attempt in 0..16 {
            let path = root.join(format!(
                "parenox-test-{}-{started}-{attempt}",
                process::id()
            ));
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "test run directory already exists",
        ))
    }
}

impl Drop for RunDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
