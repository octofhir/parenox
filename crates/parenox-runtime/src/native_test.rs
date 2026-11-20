use std::ffi::OsString;
use std::fs;

use super::native::{Clojurust, validate_namespaces};
use super::project::RunDirectory;
use super::report::{CommandExecutor, Phase, RunError, TestReport, execute_report};
use super::test_run::{TestSelection, decode_test, qualified_var};

const RUNNER: &str = include_str!("clojurust_test.cljrs");

impl Clojurust {
    /// Run selected tests and return counts and findings from native clojure.test events.
    pub fn test_report(
        &self,
        selection: TestSelection<'_>,
        detailed: bool,
    ) -> Result<TestReport, RunError> {
        self.test_report_with(selection, detailed, None)
    }

    /// Run tests with an optional caller-owned executor for the runtime command.
    pub fn test_report_with(
        &self,
        selection: TestSelection<'_>,
        detailed: bool,
        executor: Option<CommandExecutor<'_>>,
    ) -> Result<TestReport, RunError> {
        let (mode, names) = match selection {
            TestSelection::Namespaces(names) => {
                validate_namespaces(names)?;
                ("namespaces", names)
            }
            TestSelection::Vars(names) => {
                for name in names {
                    if !qualified_var(name) {
                        return Err(RunError::new(
                            Phase::Selection,
                            format!("Expected a qualified Clojure test var, got {name:?}"),
                        ));
                    }
                }
                ("vars", names)
            }
        };
        let run = RunDirectory::create("native-test").map_err(|error| {
            RunError::new(
                Phase::Launch,
                format!("create native test run directory: {error}"),
            )
        })?;
        let script = run.0.join("test.cljrs");
        let result = run.0.join("result");
        fs::write(&script, RUNNER).map_err(|error| {
            RunError::new(Phase::Launch, format!("write native test runner: {error}"))
        })?;
        let args: Vec<_> = std::iter::once(result.as_os_str().to_owned())
            .chain(std::iter::once(OsString::from(mode)))
            .chain(names.iter().map(OsString::from))
            .collect();
        let command = self.run_command(&script, &args)?;
        let result = execute_report(
            command,
            &result,
            "test",
            "clojurust",
            "cljrs",
            detailed,
            executor,
        )?;
        decode_test(&result, "clojurust")
    }
}
