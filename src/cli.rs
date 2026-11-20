use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::extern_diff_cli::DiffOptions;

#[derive(Parser)]
#[command(
    about = "Clojure checker, formatter, language server and runtime CLI",
    version
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Check Clojure files or a configured project.
    Check {
        #[command(flatten)]
        options: CheckOptions,
        /// Accept analysis-coverage warnings and report incomplete checks.
        #[arg(long)]
        allow_incomplete: bool,
    },
    /// Evaluate Clojure forms.
    Eval(EvalOptions),
    /// Run a Clojure script, namespace main or deps.edn alias.
    Run(RunOptions),
    /// Run clojure.test namespaces on the project's JVM.
    Test(TestOptions),
    /// Send explicit requests to an existing local nREPL server.
    Nrepl(crate::nrepl::Options),
    /// Find a source var or local binding definition.
    Definition(DefinitionOptions),
    /// Check library bodies and validate the extern promises they ship.
    ValidateExterns(CheckOptions),
    /// Write checked public signatures and literal values as EDN to stdout.
    ExportExterns(CheckOptions),
    /// Propose named spec contracts, with source and inferred types, as EDN.
    ProposeContracts {
        #[command(flatten)]
        options: CheckOptions,
        /// Select a fully qualified spec keyword. Repeat to select multiple specs.
        #[arg(long = "spec", value_name = "KEYWORD")]
        specs: Vec<String>,
        /// Emit the standalone extern map for generated entries.
        #[arg(long)]
        externs_only: bool,
    },
    /// Format Clojure and EDN files using parenox.edn.
    Fmt(FmtOptions),
    /// Compare the public extern promises of two releases.
    DiffExterns(DiffOptions),
    /// Print the lossless syntax tree of a source file.
    Syntax {
        #[arg(value_name = "FILE")]
        path: String,
    },
    /// Check in-memory source snapshots over the JSON Lines protocol.
    Serve,
    /// Serve editor diagnostics and navigation using LSP over stdio.
    Lsp(LspOptions),
    /// Measure repeated checks of a single source file.
    Bench {
        #[arg(value_name = "FILE")]
        path: String,
        #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u32).range(1..=1_000_000))]
        iterations: u32,
    },
}

#[derive(Args)]
pub(crate) struct JvmOptions {
    /// Project directory. JVM commands and --deps-paths use its deps.edn.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub(crate) project: PathBuf,
    /// Select a deps.edn alias. Repeat to combine aliases in order.
    #[arg(long = "alias", value_name = "NAME")]
    pub(crate) aliases: Vec<String>,
    /// Clojure CLI executable (defaults to clojure).
    #[arg(long, value_name = "EXECUTABLE")]
    pub(crate) clojure: Option<PathBuf>,
}

#[derive(Args)]
pub(crate) struct EvalOptions {
    #[command(flatten)]
    pub(crate) jvm: JvmOptions,
    #[command(flatten)]
    pub(crate) runtime: BackendOptions,
    /// Clojure source to evaluate. Repeat to evaluate in the same process.
    #[arg(
        long = "expr",
        value_name = "SOURCE",
        required = true,
        allow_hyphen_values = true
    )]
    pub(crate) expressions: Vec<String>,
    /// Terminate evaluation after this many milliseconds.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub(crate) timeout_ms: Option<u64>,
    /// Result format. Program output goes to stderr.
    #[arg(long, value_enum, default_value_t = RuntimeFormat::Text)]
    pub(crate) format: RuntimeFormat,
}

#[derive(Args)]
pub(crate) struct RunOptions {
    #[command(flatten)]
    pub(crate) jvm: JvmOptions,
    #[command(flatten)]
    pub(crate) runtime: BackendOptions,
    /// Invoke the namespace's -main.
    #[arg(long, value_name = "NAMESPACE", conflicts_with = "script")]
    pub(crate) main: Option<String>,
    /// Terminate execution if the run exceeds this many milliseconds.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub(crate) timeout_ms: Option<u64>,
    /// Script path relative to the project directory.
    #[arg(value_name = "SCRIPT")]
    pub(crate) script: Option<PathBuf>,
    /// Arguments passed to the program.
    #[arg(last = true, value_name = "ARGS")]
    pub(crate) args: Vec<OsString>,
}

#[derive(Args)]
pub(crate) struct BackendOptions {
    /// Clojure runtime to use.
    #[arg(long, value_enum, default_value_t = RuntimeBackend::Jvm)]
    pub(crate) backend: RuntimeBackend,
    /// External clojurust executable (defaults to cljrs).
    #[arg(long, value_name = "EXECUTABLE")]
    pub(crate) cljrs: Option<PathBuf>,
    /// Source root for clojurust namespace loading. Repeat to add more roots.
    #[arg(long = "src-path", value_name = "DIR")]
    pub(crate) source_paths: Vec<PathBuf>,
    /// Use local source roots from deps.edn for clojurust, including selected alias paths.
    #[arg(long, conflicts_with = "source_paths")]
    pub(crate) deps_paths: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum RuntimeBackend {
    Jvm,
    Clojurust,
    Parenox,
}

#[derive(Args)]
pub(crate) struct TestOptions {
    #[command(flatten)]
    pub(crate) jvm: JvmOptions,
    #[command(flatten)]
    pub(crate) runtime: BackendOptions,
    /// Terminate execution if the tests exceed this many milliseconds.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub(crate) timeout_ms: Option<u64>,
    /// Test report format. JSON sends the runtime log to stderr.
    #[arg(long, value_enum, default_value_t = RuntimeFormat::Text)]
    pub(crate) format: RuntimeFormat,
    /// Run a test var. Repeat to select multiple vars. The JVM runs fixtures.
    #[arg(
        long = "var",
        value_name = "NAMESPACE/NAME",
        conflicts_with = "namespaces"
    )]
    pub(crate) vars: Vec<String>,
    /// Find conventional *_test namespaces in the selected runtime's source roots.
    #[arg(long, conflicts_with_all = ["namespaces", "vars"])]
    pub(crate) discover: bool,
    /// Namespaces to require and run with clojure.test.
    #[arg(value_name = "NAMESPACE", required_unless_present_any = ["vars", "discover"])]
    pub(crate) namespaces: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum RuntimeFormat {
    Text,
    Json,
}

#[derive(Args, Clone, Default)]
pub(crate) struct ProjectOptions {
    /// Source files or directories; defaults to parenox.edn or deps.edn paths.
    #[arg(value_name = "PATH")]
    pub(crate) paths: Vec<String>,
    /// Read project paths and externs from this EDN configuration.
    #[arg(long, value_name = "FILE")]
    pub(crate) config: Option<String>,
    /// Read source roots from this deps.edn file.
    #[arg(long, value_name = "FILE", conflicts_with = "paths")]
    pub(crate) deps: Option<String>,
    /// Select a local deps.edn alias. Repeat to combine aliases in order.
    #[arg(long = "alias", value_name = "NAME", conflicts_with = "paths")]
    pub(crate) aliases: Vec<String>,
    /// Use the source directories in a file produced by clojure -Spath.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["paths", "deps", "aliases"])]
    pub(crate) classpath: Option<String>,
}

#[derive(Args)]
pub(crate) struct DefinitionOptions {
    #[command(flatten)]
    pub(crate) project: ProjectOptions,
    /// Source file containing the reference.
    #[arg(long, value_name = "FILE")]
    pub(crate) file: String,
    /// Zero-based UTF-8 byte offset inside the reference.
    #[arg(long)]
    pub(crate) offset: usize,
}

#[derive(Args)]
pub(crate) struct CheckOptions {
    #[command(flatten)]
    pub(crate) project: ProjectOptions,
    /// Diagnostic format (stderr for pretty and for all export diagnostics).
    #[arg(long, value_enum, default_value_t = Format::Pretty)]
    format: Format,
    /// Shorthand for --format json.
    #[arg(long, conflicts_with = "format")]
    json: bool,
    /// Control ANSI color in pretty diagnostics.
    #[arg(long, value_enum, default_value_t = Color::Auto)]
    pub(crate) color: Color,
    /// Load an extern declaration file; repeat to load multiple files.
    #[arg(long, value_name = "FILE")]
    pub(crate) externs: Vec<String>,
    /// Load trusted dependencies separately from the externs being validated.
    #[arg(long, value_name = "FILE")]
    pub(crate) dependency_externs: Vec<String>,
}

impl CheckOptions {
    pub(crate) fn format(&self) -> Format {
        if self.json { Format::Json } else { self.format }
    }
}

#[derive(Args)]
pub(crate) struct LspOptions {
    #[command(flatten)]
    pub(crate) project: ProjectOptions,
    /// Use a pinned compact EDN export for ClojureDocs requests and hover.
    #[arg(long, value_name = "FILE")]
    pub(crate) clojuredocs: Option<PathBuf>,
    /// Use cached ClojureDocs without downloading updates.
    #[arg(long)]
    pub(crate) clojuredocs_offline: bool,
}

#[derive(Args)]
pub(crate) struct FmtOptions {
    #[command(flatten)]
    pub(crate) project: ProjectOptions,
    /// Exit 1 if formatting would change any file; do not write.
    #[arg(long)]
    pub(crate) check: bool,
    /// Update source files instead of printing one formatted file to stdout.
    #[arg(long, conflicts_with_all = ["check", "classpath"])]
    pub(crate) write: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Format {
    Pretty,
    Plain,
    Json,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Color {
    Auto,
    Always,
    Never,
}
