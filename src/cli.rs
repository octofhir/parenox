use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::extern_diff_cli::DiffOptions;

#[derive(Parser)]
#[command(about = "Experimental source-only Clojure checker", version)]
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
    /// Run clojure.test namespaces on the project's JVM.
    Test(TestOptions),
    /// Find the definition of a qualified source var.
    Definition(DefinitionOptions),
    /// Check library bodies and validate the extern promises they ship.
    ValidateExterns(CheckOptions),
    /// Write checked public signatures and literal values as EDN to stdout.
    ExportExterns(CheckOptions),
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
    Lsp(ProjectOptions),
    /// Measure repeated checks of a single source file.
    Bench {
        #[arg(value_name = "FILE")]
        path: String,
        #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u32).range(1..=1_000_000))]
        iterations: u32,
    },
}

#[derive(Args)]
pub(crate) struct TestOptions {
    /// Project directory containing deps.edn.
    #[arg(long, value_name = "DIR", default_value = ".")]
    pub(crate) project: PathBuf,
    /// Select a Clojure CLI alias. Repeat to combine aliases in order.
    #[arg(long = "alias", value_name = "NAME")]
    pub(crate) aliases: Vec<String>,
    /// Clojure CLI executable.
    #[arg(long, value_name = "EXECUTABLE", default_value = "clojure")]
    pub(crate) clojure: PathBuf,
    /// Test report format. JSON sends the runtime log to stderr.
    #[arg(long, value_enum, default_value_t = TestFormat::Text)]
    pub(crate) format: TestFormat,
    /// Run a test var with its fixtures. Repeat to select multiple vars.
    #[arg(
        long = "var",
        value_name = "NAMESPACE/NAME",
        conflicts_with = "namespaces"
    )]
    pub(crate) vars: Vec<String>,
    /// Find conventional *_test.clj and *_test.cljc namespaces in project source roots.
    #[arg(long, conflicts_with_all = ["namespaces", "vars"])]
    pub(crate) discover: bool,
    /// Namespaces to require and run with clojure.test.
    #[arg(value_name = "NAMESPACE", required_unless_present_any = ["vars", "discover"])]
    pub(crate) namespaces: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum TestFormat {
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
