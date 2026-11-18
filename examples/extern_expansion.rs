//! Measure cold checks and verified exports of small named spec graphs.
#[path = "support/extern_expansion.rs"]
mod fixture;

use std::time::{Duration, Instant};

use clap::Parser;
use fixture::{Shape, library};
use parenox::{Session, Source};

#[derive(Parser)]
struct Options {
    #[arg(value_enum)]
    shape: Shape,
    levels: usize,
    #[arg(default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..))]
    iterations: u32,
    #[arg(long)]
    emit_source: bool,
}

fn main() {
    let options = Options::parse();
    let source = library(options.shape, options.levels);
    if options.emit_source {
        print!("{source}");
        return;
    }
    let sources = [Source {
        path: "expansion.clj",
        source: &source,
    }];
    let mut checks = Vec::new();
    let mut exports = Vec::new();
    let mut output = None;
    for _ in 0..options.iterations {
        let start = Instant::now();
        let report = Session::default().check_project_with_externs(&sources, &[]);
        checks.push(start.elapsed());
        assert!(report.ok && report.complete, "{report:#?}");

        let start = Instant::now();
        let result = Session::default().export_externs(&sources, &[]);
        exports.push(start.elapsed());
        output = Some(result);
    }
    let result = output.unwrap();
    println!(
        "{} levels, {} iterations, {} source bytes, {} output bytes, median check {:.3} ms, median export {:.3} ms",
        options.levels,
        options.iterations,
        source.len(),
        result.edn.as_ref().map_or(0, String::len),
        median_ms(&mut checks),
        median_ms(&mut exports),
    );
    for diagnostic in &result.report.files[0].diagnostics {
        println!("{}: {}", diagnostic.code, diagnostic.message);
    }
}

fn median_ms(samples: &mut [Duration]) -> f64 {
    samples.sort_unstable();
    samples[samples.len() / 2].as_secs_f64() * 1000.0
}
