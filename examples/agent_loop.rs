//! End-to-end agent scenarios over the CLI's JSON Lines protocol.
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Agent {
    child: Child,
    input: BufWriter<ChildStdin>,
    output: BufReader<ChildStdout>,
    sequence: u64,
}

impl Agent {
    fn start(binary: &Path, root: &Path) -> Result<Self> {
        let mut child = Command::new(binary)
            .arg("serve")
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        Ok(Self {
            input: BufWriter::new(child.stdin.take().unwrap()),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
            sequence: 0,
        })
    }

    fn send(&mut self, payload: &mut Value) -> Result<(Value, f64)> {
        self.sequence += 1;
        payload["id"] = json!(self.sequence);
        let started = Instant::now();
        serde_json::to_writer(&mut self.input, payload)?;
        writeln!(self.input)?;
        self.input.flush()?;
        let mut line = String::new();
        if self.output.read_line(&mut line)? == 0 {
            return Err("Checker exited without a response".into());
        }
        let mut response: Value = serde_json::from_str(&line)?;
        let elapsed = started.elapsed().as_secs_f64() * 1_000_000.0;
        assert_eq!(response["id"], self.sequence, "{response}");
        assert!(response["result"].is_object(), "{response}");
        Ok((response["result"].take(), elapsed))
    }

    fn file(&mut self, path: &str, source: &str) -> Result<(Value, f64)> {
        self.send(&mut json!({"path": path, "source": source}))
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        // Reap the child on success, I/O failure and assertion panic alike.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn summarize(mut samples: Vec<f64>) -> Value {
    samples.sort_unstable_by(f64::total_cmp);
    let n = samples.len();
    let round = |value: f64| (value * 10.0).round() / 10.0;
    json!({
        "p50_us": round((samples[(n - 1) / 2] + samples[n / 2]) / 2.0),
        "p95_us": round(samples[(n * 95).div_ceil(100) - 1]),
        "max_us": round(samples[n - 1]),
    })
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ProjectTransport {
    #[default]
    Snapshot,
    Delta,
}

impl ProjectTransport {
    fn open(self, agent: &mut Agent, snapshot: &mut Value) -> Result<(Value, f64)> {
        if self == Self::Snapshot {
            return agent.send(snapshot);
        }
        snapshot["op"] = json!("open");
        let result = agent.send(snapshot);
        snapshot.as_object_mut().unwrap().remove("op");
        result
    }

    fn check(
        self,
        agent: &mut Agent,
        snapshot: &mut Value,
        changed: &[usize],
    ) -> Result<(Value, f64)> {
        match self {
            Self::Snapshot => agent.send(snapshot),
            Self::Delta => {
                let changes: Vec<_> = changed
                    .iter()
                    .map(|&index| &snapshot["files"][index])
                    .collect();
                agent.send(&mut json!({"op": "update", "changes": changes}))
            }
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Delta => "delta",
        }
    }
}

struct ClinicOptions<'a> {
    project: &'a str,
    iterations: usize,
    extra_externs: usize,
    shifted_functions: usize,
    extra_consumers: usize,
    transport: ProjectTransport,
}

fn clinic(agent: &mut Agent, root: &Path, options: ClinicOptions<'_>) -> Result<Value> {
    let ClinicOptions {
        project,
        iterations,
        extra_externs,
        shifted_functions,
        extra_consumers,
        transport,
    } = options;
    let total_functions = 3 + shifted_functions + extra_consumers;
    let files: Vec<_> = ["http.clj", "model.clj", "service.clj", "store.clj"]
        .into_iter()
        .map(|name| {
            let path = Path::new(project).join(name);
            let mut source = fs::read_to_string(root.join(&path))?;
            if name == "service.clj" {
                for index in 0..shifted_functions {
                    writeln!(source,
                        "\n(defn bench-label-{index} {{:t/sig '[::model/patient :-> string?]}} [p]\n  (str/upper-case (or (:name p) \"Unnamed\")))")?;
                }
                if extra_consumers > 0 {
                    writeln!(source, "\n(def bench-unused :cold)")?;
                }
            }
            if name == "http.clj" {
                for index in 0..extra_consumers {
                    writeln!(source,
                        "\n(defn bench-view-{index} {{:t/sig '[::model/patient :-> string?]}} [p]\n  (service/display-name p))")?;
                }
            }
            Ok(json!({"path": path, "source": source}))
        })
        .collect::<Result<_>>()?;
    let extern_path = Path::new(project).join("externs.edn");
    let mut externs = fs::read_to_string(root.join(&extern_path))?;
    if extra_externs > 0 {
        let original = externs
            .trim_end()
            .strip_suffix('}')
            .ok_or("Expected an extern EDN map")?;
        externs.truncate(original.len());
        for index in 0..extra_externs {
            writeln!(
                externs,
                " bench.library/operation-{index} [(clojure.spec.alpha/nilable :clinic.model/patient) :-> string?]"
            )?;
        }
        externs.push('}');
    }
    let mut payload = json!({"files": files, "externs": {
        "path": extern_path, "source": externs,
    }});
    let (initial, first_us) = transport.open(agent, &mut payload)?;
    let errors: Vec<_> = initial["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|file| {
            file["diagnostics"].as_array().unwrap().iter().map(|d| {
                json!({
                    "file": file["path"], "line": d["span"]["start"]["line"],
                    "code": d["code"], "message": d["message"],
                })
            })
        })
        .collect();
    assert_eq!(errors.len(), 3, "{initial}");
    assert_eq!(initial["complete"], true, "{initial}");
    let mut fixes = Vec::new();
    for (index, file) in payload["files"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        let source = file["source"].as_str().unwrap();
        let fixed = source
            .replace(
                "(str/upper-case (:name patient))",
                "(str/upper-case (or (:name patient) \"Unknown\"))",
            )
            .replace("(store/find-patient 42)", "(store/find-patient id)")
            .replace("(str/starts-with? 42", "(str/starts-with? id");
        if fixed != source {
            file["source"] = json!(fixed);
            fixes.push(index);
        }
    }
    let (fixed, fix_us) = transport.check(agent, &mut payload, &fixes)?;
    assert_eq!(fixed["ok"], true, "{fixed}");
    let checks = fixed["stats"]["checked_functions"].as_u64().unwrap() as usize;
    assert!((2..total_functions).contains(&checks), "{fixed}");
    let service = payload["files"]
        .as_array()
        .unwrap()
        .iter()
        .position(|f| f["path"].as_str().unwrap().ends_with("service.clj"))
        .unwrap();
    let mut warm_times = Vec::with_capacity(iterations);
    let mut edit_times = Vec::with_capacity(iterations);
    let mut edited = Value::Null;
    for index in 0..iterations {
        let (warm, elapsed) = transport.check(agent, &mut payload, &[])?;
        assert_eq!(warm["ok"], true, "{warm}");
        assert_eq!(warm["stats"]["checked_functions"], 0, "{warm}");
        assert_eq!(warm["stats"]["reused_functions"], total_functions, "{warm}");
        assert_eq!(warm["stats"]["parsed_files"], 0, "{warm}");
        warm_times.push(elapsed);
        let (before, after) = if extra_consumers == 0 {
            ("Unknown", "Anonymous")
        } else {
            ("bench-unused :cold", "bench-unused :hot")
        };
        let (old, new) = if index % 2 == 0 {
            (before, after)
        } else {
            (after, before)
        };
        let source = &mut payload["files"][service]["source"];
        *source = json!(source.as_str().unwrap().replace(old, new));
        let (report, elapsed) = transport.check(agent, &mut payload, &[service])?;
        assert_eq!(report["ok"], true, "{report}");
        let checks = report["stats"]["checked_functions"].as_u64().unwrap() as usize;
        let maximum = if extra_consumers == 0 {
            1 + shifted_functions
        } else {
            2 + shifted_functions + extra_consumers
        };
        assert!((1..=maximum).contains(&checks), "{report}");
        assert_eq!(
            report["stats"]["reused_functions"],
            total_functions - checks,
            "{report}"
        );
        edit_times.push(elapsed);
        edited = report;
    }
    let source = &mut payload["files"][service]["source"];
    *source = json!(source.as_str().unwrap().replacen(
        "'[::model/patient :-> string?]",
        "'[::model/patient :-> number?]",
        1,
    ));
    let (changed, change_us) = transport.check(agent, &mut payload, &[service])?;
    assert_eq!(changed["ok"], false, "{changed}");
    let checks = changed["stats"]["checked_functions"].as_u64().unwrap() as usize;
    assert!(
        (2 + extra_consumers..total_functions).contains(&checks),
        "{changed}"
    );
    assert!(
        changed["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["ok"] == false && f["path"].as_str().unwrap().ends_with("http.clj")),
        "{changed}"
    );
    let mut report = json!({
        "scenario": "clinic: 4 namespaces + 1 declared library contract",
        "transport": transport.name(),
        "extra_externs": extra_externs,
        "shifted_functions": shifted_functions,
        "extra_consumers": extra_consumers,
        "iterations": iterations, "initial_errors": errors,
        "first_round_trip_us": first_us, "fix_round_trip_us": fix_us,
        "after_fix": fixed["stats"],
        "warm_round_trip": summarize(warm_times),
        "signature_change": {"round_trip_us": change_us, "stats": changed["stats"]},
        "scope": "Release CLI process; Rust JSON encoding, pipes and response decoding. Local fixture, not a production-scale benchmark.",
    });
    let edit = if extra_consumers == 0 {
        "body_edit"
    } else {
        "unused_export_edit"
    };
    report[format!("{edit}_round_trip")] = summarize(edit_times);
    report[edit] = edited["stats"].take();
    Ok(report)
}

fn synthetic(
    agent: &mut Agent,
    root: &Path,
    binary: &Path,
    iterations: usize,
    functions: usize,
) -> Result<Value> {
    let bad = "(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))\n\
               (defn label {:t/sig '[(s/nilable string?) :-> string?]} [name] (str/upper-case name))";
    let (report, _) = agent.file("edit.clj", bad)?;
    assert_eq!(report["ok"], false, "{report}");
    assert_eq!(report["complete"], true, "{report}");
    assert_eq!(report["diagnostics"][0]["code"], "PNX2322", "{report}");
    let fixed = bad.replace(
        "(str/upper-case name)",
        "(str/upper-case (or name \"Unknown\"))",
    );
    let (report, _) = agent.file("edit.clj", &fixed)?;
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["stats"]["checked_functions"], 1, "{report}");
    let (report, _) = agent.file("edit.clj", &fixed)?;
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["stats"]["reused_functions"], 1, "{report}");

    let small = fs::read_to_string(root.join("examples/good.clj"))?;
    let (report, small_first) = agent.file("small.clj", &small)?;
    assert_eq!(report["ok"], true, "{report}");
    let mut small_times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let (report, elapsed) = agent.file("small.clj", &small)?;
        assert_eq!(report["ok"], true, "{report}");
        assert_eq!(report["stats"]["reused_functions"], 4, "{report}");
        small_times.push(elapsed);
    }
    let mut large = String::from("(ns generated)\n");
    for index in 0..functions {
        large.push_str(&format!(
            "(defn f{index} {{:t/sig '[number? :-> number?]}} [x] (+ x 1))\n"
        ));
    }
    let (report, large_first) = agent.file("large.clj", &large)?;
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["stats"]["checked_functions"], functions, "{report}");
    let mut warm_times = Vec::with_capacity(iterations);
    let mut edit_times = Vec::with_capacity(iterations);
    for index in 0..iterations {
        let (report, elapsed) = agent.file("large.clj", &large)?;
        assert_eq!(report["ok"], true, "{report}");
        assert_eq!(report["stats"]["reused_functions"], functions, "{report}");
        warm_times.push(elapsed);
        // A same-width edit in the last body preserves all other source ranges.
        let position = large.rfind("(+ x ").unwrap() + "(+ x ".len();
        large.replace_range(
            position..position + 1,
            if index % 2 == 0 { "2" } else { "1" },
        );
        let (report, elapsed) = agent.file("large.clj", &large)?;
        assert_eq!(report["ok"], true, "{report}");
        assert_eq!(report["stats"]["checked_functions"], 1, "{report}");
        assert_eq!(
            report["stats"]["reused_functions"],
            functions - 1,
            "{report}"
        );
        edit_times.push(elapsed);
    }
    let mut cold_times = Vec::new();
    for _ in 0..iterations.min(30) {
        let started = Instant::now();
        let output = Command::new(binary)
            .args(["check", "examples/good.clj", "--json"])
            .current_dir(root)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(report["ok"], true, "{report}");
        cold_times.push(started.elapsed().as_secs_f64() * 1_000_000.0);
    }
    Ok(json!({
        "iterations": iterations, "agent_edit_fix_recheck": "passed",
        "small_file": {"bytes": small.len(), "functions": 4, "first_round_trip_us": small_first,
            "warm_round_trip": summarize(small_times)},
        "generated_file": {"bytes": large.len(), "functions": functions,
            "first_round_trip_us": large_first, "warm_round_trip": summarize(warm_times),
            "single_body_edit_round_trip": summarize(edit_times)},
        "fresh_cli_process": {"iterations": iterations.min(30), "timings": summarize(cold_times)},
        "scope": "Synthetic single-file fixtures. Rust JSON encoding, pipes, full source transfer and response decoding. Fresh CLI includes process startup and file I/O. No JVM or macro expansion.",
    }))
}

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut binary = PathBuf::from("target/release/parenox");
    let mut project = None;
    let mut iterations = 100;
    let mut functions = 1000;
    let mut extra_externs = 0;
    let mut shifted_functions = 0;
    let mut extra_consumers = 0;
    let mut project_transport = ProjectTransport::default();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {flag}"))?;
        match flag.as_str() {
            "--binary" => binary = value.into(),
            "--project" => project = Some(value),
            "--iterations" => iterations = value.parse::<usize>()?,
            "--functions" => functions = value.parse::<usize>()?,
            "--extra-externs" => extra_externs = value.parse::<usize>()?,
            "--shifted-functions" => shifted_functions = value.parse::<usize>()?,
            "--extra-consumers" => extra_consumers = value.parse::<usize>()?,
            "--project-transport" => {
                project_transport = match value.as_str() {
                    "snapshot" => ProjectTransport::Snapshot,
                    "delta" => ProjectTransport::Delta,
                    _ => return Err("Project transport must be snapshot or delta".into()),
                }
            }

            _ => return Err(format!("Unknown option {flag}").into()),
        }
    }
    if iterations == 0 || functions == 0 {
        return Err("Iterations and functions must be positive".into());
    }
    if (extra_externs != 0
        || shifted_functions != 0
        || project_transport == ProjectTransport::Delta)
        && project.is_none()
    {
        return Err("Project fixtures and delta transport require --project".into());
    }
    let binary = root.join(binary);
    let mut agent = Agent::start(&binary, root)?;
    let mut report = if let Some(project) = project {
        clinic(
            &mut agent,
            root,
            ClinicOptions {
                project: &project,
                iterations,
                extra_externs,
                shifted_functions,
                extra_consumers,
                transport: project_transport,
            },
        )?
    } else {
        synthetic(&mut agent, root, &binary, iterations, functions)?
    };
    report["platform"] = json!({"os": std::env::consts::OS, "arch": std::env::consts::ARCH});
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
