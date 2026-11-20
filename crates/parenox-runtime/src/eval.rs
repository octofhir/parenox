use super::jvm::JvmProject;
use super::report::{CommandExecutor, Phase, RunError, optional_text, read_run_error};
use super::single_form;

const RUNNER: &str = include_str!("clojure_eval.clj");

impl JvmProject {
    /// Evaluate expressions in one JVM and return the last printed value.
    /// The subprocess's output is forwarded to stderr.
    pub fn eval(&self, expressions: &[String]) -> Result<String, RunError> {
        self.eval_with(expressions, None)
    }

    /// Evaluate with an optional caller-owned executor for the runtime command.
    pub fn eval_with(
        &self,
        expressions: &[String],
        executor: Option<CommandExecutor<'_>>,
    ) -> Result<String, RunError> {
        let runner = self.report_runner("eval")?;
        let result = runner.run(
            RUNNER,
            expressions.iter().map(String::as_str),
            true,
            executor,
        )?;
        decode_eval(&result, "Clojure")
    }
}

pub(super) fn decode_eval(report: &str, runtime: &str) -> Result<String, RunError> {
    let invalid = || {
        RunError::new(
            Phase::Report,
            format!("{runtime} returned an invalid eval report"),
        )
    };
    let form = single_form(report).ok_or_else(invalid)?;
    let fields = form.vector().ok_or_else(invalid)?;
    if let [marker, value] = fields
        && marker.keyword() == Some(":value")
    {
        return optional_text(value).flatten().ok_or_else(invalid);
    }
    Err(read_run_error(report, fields).unwrap_or_else(invalid))
}
