use std::fmt::Write;
use std::fs;
use std::path::Path;

use parenox_frontend::reader::{self, Conditionals};

use super::eval::decode_eval;
use super::native::Clojurust;
use super::project::RunDirectory;
use super::report::{CommandExecutor, Phase, RunError, execute_report};

impl Clojurust {
    /// Evaluate input files in one clojurust namespace environment.
    /// The last printed value is read separately from the program log.
    pub fn eval(&self, expressions: &[String]) -> Result<String, RunError> {
        self.eval_with(expressions, None)
    }

    /// Evaluate with an optional caller-owned executor for the runtime command.
    pub fn eval_with(
        &self,
        expressions: &[String],
        executor: Option<CommandExecutor<'_>>,
    ) -> Result<String, RunError> {
        let run = RunDirectory::create("native-eval").map_err(|error| {
            RunError::new(
                Phase::Launch,
                format!("create native eval run directory: {error}"),
            )
        })?;
        let result = run.0.join("result");
        let result_literal = path_literal(&result)?;
        let path = run.0.join("eval.cljrs");

        // A trailing input with no forms leaves the previous value intact.
        // Let clojurust handle syntax that Parenox cannot read.
        let count = expressions
            .iter()
            .rposition(|source| {
                !reader::read_with(source, Conditionals::Preserve)
                    .is_ok_and(|forms| forms.is_empty())
            })
            .map_or(0, |last| last + 1);
        let mut script = format!("(clojure.core/spit {result_literal} \":started\")\n(try\n");
        let mut last = "nil".to_owned();
        for (index, source) in expressions[..count].iter().enumerate() {
            let input = run.0.join(format!("expr-{}.cljrs", index + 1));
            write_source(&input, source)?;
            let load = format!("(load-file {})", path_literal(&input)?);
            if index + 1 == count {
                last = load;
            } else {
                writeln!(script, "  {load}").unwrap();
            }
        }
        // Do not bind driver locals while load-file evaluates user forms.
        // Change namespace before cljrs checks for a script's -main.
        write!(
            script,
            r#"  (clojure.core/spit {result_literal}
    (clojure.core/pr-str [:value (clojure.core/pr-str {last})]))
  (catch Throwable error
    (let [cause (loop [cause error remaining 63]
                  (if (clojure.core/zero? remaining)
                    cause
                    (clojure.core/if-let [next (clojure.core/ex-cause cause)]
                      (recur next (clojure.core/dec remaining))
                      cause)))
          message (or (clojure.core/ex-message cause) (clojure.core/pr-str cause))]
      (clojure.core/spit {result_literal}
        (clojure.core/pr-str [:run-error :execution message nil nil nil nil])))))
(in-ns (clojure.core/gensym "parenox.runtime.eval-done-"))
(def -main (fn [& _] nil))
"#,
        )
        .unwrap();
        write_source(&path, &script)?;

        let command = self.run_command(&path, &[])?;
        let report = execute_report(
            command,
            &result,
            "eval",
            "clojurust",
            "cljrs",
            true,
            executor,
        )?;
        decode_eval(&report, "clojurust")
    }
}

fn write_source(path: &Path, content: &str) -> Result<(), RunError> {
    fs::write(path, content).map_err(|error| {
        RunError::new(
            Phase::Launch,
            format!("write native eval source {}: {error}", path.display()),
        )
    })
}

fn path_literal(path: &Path) -> Result<String, RunError> {
    let path = path.to_str().ok_or_else(|| {
        RunError::new(Phase::Launch, "native eval requires a UTF-8 temporary path")
    })?;
    let mut quoted = String::with_capacity(path.len() + 2);
    quoted.push('"');
    for character in path.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    Ok(quoted)
}
