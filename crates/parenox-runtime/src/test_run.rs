use super::jvm::JvmProject;
use super::report::{CommandExecutor, Phase, RunError, TestReport};
use super::{namespace_symbol, single_form};

const RUNNER: &str = include_str!("clojure_test.clj");

pub enum TestSelection<'a> {
    Namespaces(&'a [String]),
    Vars(&'a [String]),
}

impl JvmProject {
    /// Run selected clojure.test namespaces or vars in the project JVM.
    /// Detailed reports include exception details and send the test log to stderr.
    pub fn test(
        &self,
        selection: TestSelection<'_>,
        detailed: bool,
    ) -> Result<TestReport, RunError> {
        self.test_with(selection, detailed, None)
    }

    /// Run tests with an optional caller-owned executor for the runtime command.
    pub fn test_with(
        &self,
        selection: TestSelection<'_>,
        detailed: bool,
        executor: Option<CommandExecutor<'_>>,
    ) -> Result<TestReport, RunError> {
        let runner = self.report_runner("test")?;
        let (mode, names) = match selection {
            TestSelection::Namespaces(names) => {
                for name in names {
                    if !namespace_symbol(name) {
                        return Err(RunError::new(
                            Phase::Selection,
                            format!("Expected a Clojure namespace, got {name:?}"),
                        ));
                    }
                }
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
        let format = if detailed { "json" } else { "text" };
        let args = [mode, format]
            .into_iter()
            .chain(names.iter().map(String::as_str));
        let result = runner.run(RUNNER, args, detailed, executor)?;
        decode_test(&result, "Clojure")
    }
}

pub(super) fn decode_test(result: &str, runtime: &str) -> Result<TestReport, RunError> {
    let report = TestReport::read(result).ok_or_else(|| {
        RunError::new(
            Phase::Report,
            format!("{runtime} returned an invalid test report"),
        )
    })?;
    if report.tests == 0 && report.run_error.is_none() {
        return Err(RunError::new(
            Phase::Execution,
            "no clojure.test vars ran for the requested selection",
        ));
    }
    Ok(report)
}

pub(super) fn qualified_var(name: &str) -> bool {
    name.split_once('/').is_some_and(|(namespace, local)| {
        namespace_symbol(namespace)
            && !local.is_empty()
            && (local == "/" || !local.contains(['/', ':']))
            && single_form(name).is_some_and(|form| form.symbol() == Some(name))
    })
}
