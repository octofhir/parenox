//! Wire explicit runtime requests to the CLI and its disk projects.
mod discover;
mod native_main;
mod process;

use std::borrow::Cow;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use parenox::reader::Conditionals;
use parenox_runtime::{
    Clojurust, CommandExecutor, Engine, JvmMain, JvmProject, Phase, RunError, TestReport,
    TestSelection,
};
use serde_json::json;

use crate::cli::{
    BackendOptions, EvalOptions, JvmOptions, RunOptions, RuntimeBackend, RuntimeFormat, TestOptions,
};

const NATIVE_CONDITIONALS: Conditionals<'static> =
    Conditionals::SelectFeatures(&[":cljrs", ":rust"]);

impl JvmOptions {
    fn open(&self) -> Result<JvmProject, RunError> {
        let executable = self.clojure.as_deref().unwrap_or(Path::new("clojure"));
        JvmProject::open(&self.project, executable, &self.aliases)
    }
}

impl BackendOptions {
    fn validate_jvm(&self) -> Result<(), RunError> {
        if self.deps_paths {
            return Err(RunError::new(
                Phase::Selection,
                "--deps-paths requires --backend clojurust",
            ));
        }
        if self.cljrs.is_some() || !self.source_paths.is_empty() {
            return Err(RunError::new(
                Phase::Selection,
                "--cljrs and --src-path require --backend clojurust",
            ));
        }
        Ok(())
    }

    fn open_native(&self, jvm: &JvmOptions) -> Result<Clojurust, RunError> {
        if jvm.clojure.is_some() {
            return Err(RunError::new(
                Phase::Selection,
                "--clojure requires --backend jvm",
            ));
        }
        let executable = self.cljrs.as_deref().unwrap_or(Path::new("cljrs"));
        if self.deps_paths {
            let project = jvm.project.canonicalize().map_err(|error| {
                RunError::new(
                    Phase::Project,
                    format!("{}: {error}", jvm.project.display()),
                )
            })?;
            let roots = crate::config::deps_paths(&project.join("deps.edn"), &jvm.aliases)
                .map_err(|message| RunError::new(Phase::Project, message))?;
            let roots: Vec<_> = roots.into_iter().map(PathBuf::from).collect();
            return Clojurust::open(&project, executable, &roots);
        }
        if !jvm.aliases.is_empty() {
            return Err(RunError::new(
                Phase::Selection,
                "--alias with clojurust requires --deps-paths",
            ));
        }
        Clojurust::open(&jvm.project, executable, &self.source_paths)
    }
}

pub(super) fn eval(options: EvalOptions) -> Result<u8, String> {
    let mut timed = options.timeout_ms.map(|ms| {
        let timeout = Duration::from_millis(ms);
        move |command| process::report(command, "eval", timeout, true)
    });
    let executor = timed.as_mut().map(|execute| execute as CommandExecutor<'_>);
    let result = match options.runtime.backend {
        RuntimeBackend::Jvm => options
            .runtime
            .validate_jvm()
            .and_then(|()| options.jvm.open())
            .and_then(|project| project.eval_with(&options.expressions, executor)),
        RuntimeBackend::Clojurust => options
            .runtime
            .open_native(&options.jvm)
            .and_then(|project| project.eval_with(&options.expressions, executor)),
        RuntimeBackend::Parenox => own_eval(&options),
    };
    if options.format == RuntimeFormat::Json {
        let report = match &result {
            Ok(value) => json!({"version": 1, "ok": true, "value": value}),
            Err(error) => json!({"version": 1, "ok": false, "run_error": error}),
        };
        println!("{report}");
    }
    let value = match result {
        Ok(value) => value,
        Err(error) => return report_error(error),
    };
    if options.format == RuntimeFormat::Text {
        println!("{value}");
    }
    Ok(0)
}

fn own_eval(options: &EvalOptions) -> Result<String, RunError> {
    if options.jvm.clojure.is_some()
        || !options.jvm.aliases.is_empty()
        || options.runtime.cljrs.is_some()
        || options.runtime.deps_paths
        || !options.runtime.source_paths.is_empty()
    {
        return Err(RunError::new(
            Phase::Selection,
            "--backend parenox eval does not accept external runtime or dependency options",
        ));
    }
    if !options.jvm.project.is_dir() {
        return Err(RunError::new(
            Phase::Project,
            format!(
                "{}: expected a project directory",
                options.jvm.project.display()
            ),
        ));
    }
    Engine::default().eval(
        &options.expressions,
        &mut io::stderr().lock(),
        options.timeout_ms.map(Duration::from_millis),
    )
}

pub(super) fn run(options: RunOptions) -> Result<u8, String> {
    let command = match options.runtime.backend {
        RuntimeBackend::Jvm => jvm_run(&options)?,
        RuntimeBackend::Clojurust => native_run(&options)?,
        RuntimeBackend::Parenox => return Err(own_backend_eval_only().to_string()),
    };
    process::run(
        command,
        "run",
        options.timeout_ms.map(Duration::from_millis),
    )
}

fn jvm_run(options: &RunOptions) -> Result<Command, String> {
    options
        .runtime
        .validate_jvm()
        .map_err(|error| error.to_string())?;
    let project = options.jvm.open().map_err(|error| error.to_string())?;
    let main = match (options.main.as_deref(), options.script.as_deref()) {
        (Some(name), _) => JvmMain::Namespace(name),
        (_, Some(path)) => JvmMain::Script(path),
        _ => JvmMain::Alias,
    };
    project
        .run_command(main, &options.args)
        .map_err(|error| error.to_string())
}

fn native_run(options: &RunOptions) -> Result<Command, String> {
    let project = options
        .runtime
        .open_native(&options.jvm)
        .map_err(|error| error.to_string())?;
    let selected;
    let script = if let Some(namespace) = &options.main {
        selected = native_main::resolve(&project, namespace).map_err(|error| error.to_string())?;
        &selected
    } else {
        options
            .script
            .as_deref()
            .ok_or("Specify a .cljc or .cljrs script, or --main NAMESPACE")?
    };
    project
        .run_command(script, &options.args)
        .map_err(|error| error.to_string())
}

pub(super) fn run_tests(options: TestOptions) -> Result<u8, String> {
    let detailed = options.format == RuntimeFormat::Json;
    let mut timed = options.timeout_ms.map(|ms| {
        let timeout = Duration::from_millis(ms);
        move |command| process::report(command, "test", timeout, detailed)
    });
    let executor = timed.as_mut().map(|execute| execute as CommandExecutor<'_>);
    let report = match options.runtime.backend {
        RuntimeBackend::Jvm => execute_tests(&options, detailed, executor),
        RuntimeBackend::Clojurust if !detailed && options.vars.is_empty() => {
            return native_test(&options);
        }
        RuntimeBackend::Clojurust => execute_native_tests(&options, detailed, executor),
        RuntimeBackend::Parenox => Err(own_backend_eval_only()),
    }
    .unwrap_or_else(TestReport::incomplete);
    if detailed {
        println!(
            "{}",
            serde_json::to_string(&report).map_err(|error| error.to_string())?
        );
    }
    if let Some(error) = report.run_error {
        return report_error(error);
    }
    Ok(u8::from(!report.ok))
}

fn own_backend_eval_only() -> RunError {
    RunError::new(
        Phase::Selection,
        "--backend parenox currently supports eval",
    )
}

fn report_error(error: RunError) -> Result<u8, String> {
    if let Some(code) = error.exit_code() {
        eprintln!("parenox: {error}");
        Ok(code)
    } else {
        Err(error.to_string())
    }
}

fn native_test(options: &TestOptions) -> Result<u8, String> {
    let (project, namespaces) = select_native_tests(options).map_err(|error| error.to_string())?;
    let command = project
        .test_command(&namespaces)
        .map_err(|error| error.to_string())?;
    process::run(
        command,
        "test",
        options.timeout_ms.map(Duration::from_millis),
    )
}

fn select_native_tests(options: &TestOptions) -> Result<(Clojurust, Cow<'_, [String]>), RunError> {
    let project = options.runtime.open_native(&options.jvm)?;
    let namespaces = if options.discover {
        Cow::Owned(discover::native_namespaces(&project)?)
    } else {
        Cow::Borrowed(options.namespaces.as_slice())
    };
    Ok((project, namespaces))
}

fn execute_native_tests(
    options: &TestOptions,
    detailed: bool,
    executor: Option<CommandExecutor<'_>>,
) -> Result<TestReport, RunError> {
    if options.vars.is_empty() {
        let (project, namespaces) = select_native_tests(options)?;
        project.test_report_with(TestSelection::Namespaces(&namespaces), detailed, executor)
    } else {
        options.runtime.open_native(&options.jvm)?.test_report_with(
            TestSelection::Vars(&options.vars),
            detailed,
            executor,
        )
    }
}

fn execute_tests(
    options: &TestOptions,
    detailed: bool,
    executor: Option<CommandExecutor<'_>>,
) -> Result<TestReport, RunError> {
    options.runtime.validate_jvm()?;
    let project = options.jvm.open()?;
    let discovered;
    let selection = if options.discover {
        discovered = discover::namespaces(project.root(), &options.jvm.aliases)?;
        TestSelection::Namespaces(&discovered)
    } else if options.vars.is_empty() {
        TestSelection::Namespaces(&options.namespaces)
    } else {
        TestSelection::Vars(&options.vars)
    };
    project.test_with(selection, detailed, executor)
}
