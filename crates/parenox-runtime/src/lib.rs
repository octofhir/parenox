//! Clojure execution in Parenox and through explicit external runtimes.
mod engine;
mod eval;
mod jvm;
mod native;
mod native_eval;
mod native_test;
mod project;
mod report;
mod test_run;

use parenox_frontend::reader::{Form, read};

pub use engine::Engine;
pub use jvm::{JvmMain, JvmProject};
pub use native::Clojurust;
pub use report::{CommandExecutor, Phase, RunError, TestReport};
pub use test_run::TestSelection;

fn single_form(source: &str) -> Option<Form> {
    let mut forms = read(source).ok()?;
    (forms.len() == 1).then(|| forms.remove(0))
}

pub fn namespace_symbol(name: &str) -> bool {
    !name.contains(['/', ':']) && single_form(name).is_some_and(|form| form.symbol() == Some(name))
}
