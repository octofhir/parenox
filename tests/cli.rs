use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn check_has_machine_readable_output_and_meaningful_exit_codes() {
    for (file, success) in [("examples/good.clj", true), ("examples/bad.clj", false)] {
        let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
            .args(["check", file, "--json"])
            .output()
            .unwrap();
        assert_eq!(output.status.success(), success);
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["ok"], success);
        assert_eq!(report["complete"], true);
    }
}

#[test]
fn repeated_extern_flags_load_every_file() {
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args([
            "check",
            "examples/split_externs/app.clj",
            "--json",
            "--externs",
            "examples/split_externs/queries.edn",
            "--externs",
            "examples/split_externs/formatting.edn",
        ])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{report}");
    assert_eq!(report["files"].as_array().unwrap().len(), 3, "{report}");
}

#[test]
fn extern_diff_reads_explicit_file_lists_and_configuration_snapshots() {
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args([
            "diff-externs",
            "--before-config",
            "examples/extern_diff/before/parenox.edn",
            "--after-config",
            "examples/extern_diff/after/parenox.edn",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["compatible"], false);
    assert_eq!(report["complete"], true);
    let changes = report["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 3);
    let title = &changes[2];
    assert_eq!(title["name"], "vendor.labels/title");
    assert_eq!(title["kind"], "breaking");
    assert_eq!(
        title["detail"],
        serde_json::json!({
            "path": [
                {"kind": "argument", "index": 1},
                {"kind": "field", "key": ":payload"},
                {"kind": "field", "key": ":title"}
            ],
            "reason": "type_mismatch",
            "expected": "Keyword excluding :clojure.spec.alpha/invalid",
            "actual": "String"
        })
    );
    assert_eq!(title["before"]["span"]["start"]["line"], 1);
    assert_eq!(
        title["after"]["origin"]["path"],
        "examples/extern_diff/after/api.edn"
    );
    let plain = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args([
            "diff-externs",
            "--before-config",
            "examples/extern_diff/before/parenox.edn",
            "--after-config",
            "examples/extern_diff/after/parenox.edn",
        ])
        .output()
        .unwrap();
    assert_eq!(plain.status.code(), Some(1));
    let text = String::from_utf8(plain.stdout).unwrap();
    assert!(
        text.contains(
            "argument 1 → field :payload → field :title: expected Keyword excluding :clojure.spec.alpha/invalid, got String"
        ),
        "{text}"
    );

    let queries = "examples/split_externs/queries.edn";
    let formatting = "examples/split_externs/formatting.edn";
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args([
            "diff-externs",
            "--before",
            queries,
            "--before",
            formatting,
            "--after",
            formatting,
            "--after",
            queries,
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Extern diff: compatible (0 changes)\n"
    );
}

#[test]
fn serve_flushes_responses_before_eof_and_survives_invalid_requests() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    writeln!(input, "not json").unwrap();
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line).unwrap()["error"]["code"],
        "invalid_request"
    );
    let request = serde_json::json!({"id": 7, "path": "demo.clj", "source": "(defn f {:t/sig '[:-> number?]} [] 1)"});
    for reused in [0, 1] {
        writeln!(input, "{request}").unwrap();
        line.clear();
        output.read_line(&mut line).unwrap();
        let response: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], 7);
        assert_eq!(response["result"]["ok"], true);
        assert_eq!(response["result"]["stats"]["reused_functions"], reused);
    }
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[test]
fn default_output_shows_cargo_style_source_and_suggested_edit_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args(["check", "examples/bad.clj"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(text.contains("error[PNX2322]"), "{text}");
    assert!(text.contains("--> examples/bad.clj:15:19"), "{text}");
    assert!(
        text.contains("15 |   (str/upper-case (:name patient)))"),
        "{text}"
    );
    assert!(text.contains("^^^^^^^^^^^^^^^"), "{text}");
    assert!(
        text.contains("15 +   (str/upper-case (or (:name patient) \"\")))"),
        "{text}"
    );
    assert!(text.contains("due to 3 previous errors"), "{text}");
    assert!(text.contains("Argument 1: mismatched types"), "{text}");
    assert!(text.contains("expected `Number`, found `String`"), "{text}");
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn pretty_can_force_color_or_disable_it_explicitly() {
    for (color, colored) in [("always", true), ("never", false)] {
        let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
            .args([
                "check",
                "examples/bad.clj",
                "--format=pretty",
                "--color",
                color,
            ])
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        let text = String::from_utf8(output.stderr).unwrap();
        assert_eq!(text.contains('\u{1b}'), colored, "{text}");
        if colored {
            for part in [
                "\x1b[1m\x1b[92m    Checking\x1b[0m",
                "\x1b[1m\x1b[91merror[PNX2322]\x1b[0m\x1b[1m:",
                "\x1b[1m\x1b[94m--> ",
                "\x1b[1m\x1b[91m^^^^^^^^^^^^^^^",
                "\x1b[1m\x1b[96mhelp",
                "\x1b[92m(or ",
                "\x1b[1m\x1b[91merror\x1b[0m\x1b[1m: could not check",
            ] {
                assert!(text.contains(part), "Missing {part:?} in {text:?}");
            }
        }
    }
}

#[test]
fn project_pretty_reports_a_single_colored_summary_when_forced_in_a_pipe() {
    for no_color in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_parenox"));
        command
            .args([
                "check",
                "examples/clinic",
                "--externs",
                "examples/clinic/externs.edn",
            ])
            .env("TERM", "dumb")
            .env("CLICOLOR_FORCE", "1")
            .env_remove("NO_COLOR");
        if no_color {
            command.env("NO_COLOR", "1");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let text = String::from_utf8(output.stderr).unwrap();
        assert_eq!(text.contains('\u{1b}'), !no_color);
        assert_eq!(text.matches("Checking").count(), 1);
        assert_eq!(text.matches("could not check").count(), 1);
        assert!(
            text.contains("project (4 source files) due to 3 previous errors"),
            "{text}"
        );
        assert!(!text.contains("Finished"));
    }
}

#[test]
fn plain_is_compact_and_never_contains_color_or_source_snippets() {
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args([
            "check",
            "--format",
            "plain",
            "examples/bad.clj",
            "--color=always",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("examples/bad.clj:15:19: PNX2322"), "{text}");
    assert!(text.contains("requires review"), "{text}");
    assert!(!text.contains("15 |"));
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn json_format_is_clean_and_includes_structured_reviewable_suggestions() {
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args([
            "check",
            "examples/bad.clj",
            "--format=json",
            "--color=always",
        ])
        .output()
        .unwrap();
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.contains(&0x1b));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let suggestion = &report["diagnostics"][0]["suggestions"][0];
    assert_eq!(suggestion["replacement"], "(or (:name patient) \"\")");
    assert_eq!(suggestion["applicability"], "requires-review");
    assert!(suggestion["span"]["start"]["offset"].is_number());
}

#[test]
fn invalid_or_conflicting_output_options_are_usage_errors() {
    for options in [
        vec!["--format", "xml"],
        vec!["--color", "rainbow"],
        vec!["--json", "--format", "plain"],
        vec!["--format"],
        vec!["--color"],
    ] {
        let mut args = vec!["check", "examples/good.clj"];
        args.extend(options);
        let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn subcommands_have_help_and_validate_benchmark_arguments() {
    let cli = || Command::new(env!("CARGO_BIN_EXE_parenox"));
    let help = cli().args(["export-externs", "--help"]).output().unwrap();
    assert!(help.status.success() && help.stderr.is_empty());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("--dependency-externs") && help.contains("EDN to stdout"));

    let invalid = cli()
        .args(["bench", "--iterations=0", "examples/good.clj"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());

    let valid = cli()
        .args(["bench", "--iterations=1", "examples/good.clj"])
        .output()
        .unwrap();
    assert!(valid.status.success());
    let result: serde_json::Value = serde_json::from_slice(&valid.stdout).unwrap();
    assert_eq!(result["iterations"], 1);
}

#[test]
fn successful_default_output_reports_success_without_source_noise() {
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args(["check", "examples/good.clj", "--color=never"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(
        text.contains("Finished checking `examples/good.clj` — no type errors"),
        "{text}"
    );
    assert!(!text.contains("-->"));
}

#[test]
fn syntax_command_keeps_trivia_and_reports_errors_without_losing_later_code() {
    let valid = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args(["syntax", "examples/syntax.clj"])
        .output()
        .unwrap();
    assert!(valid.status.success());
    assert!(valid.stderr.is_empty());
    let tree = String::from_utf8(valid.stdout).unwrap();
    for kind in ["Root@", "Comment@", "Discard@", "Comma@", "Map@", "Set@"] {
        assert!(tree.contains(kind), "{kind}: {tree}");
    }
    let invalid = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args(["syntax", "examples/syntax_bad.clj"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(1));
    let tree = String::from_utf8(invalid.stdout).unwrap();
    assert!(tree.contains("Error@"), "{tree}");
    assert!(tree.contains("retained"), "{tree}");
    let errors = String::from_utf8(invalid.stderr).unwrap();
    assert!(errors.contains("error[PNX9000]"), "{errors}");
    assert!(errors.contains("examples/syntax_bad.clj:4:5"), "{errors}");
}

#[test]
fn serve_rechecks_a_later_iteration_after_an_edit() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let failing = include_str!("../examples/loops_bad.clj");
    let fixed = failing.replace("(recur false \"bad\")", "(recur false 12345)");
    for (id, source, expected) in [(1, failing, 4), (2, &fixed, 3), (3, &fixed, 3)] {
        let request = serde_json::json!({"id": id, "path": "loops.clj", "source": source});
        writeln!(input, "{request}").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let response: serde_json::Value = serde_json::from_str(&line).unwrap();
        let report = &response["result"];
        assert_eq!(response["id"], id);
        assert_eq!(
            report["diagnostics"].as_array().unwrap().len(),
            expected,
            "{report}"
        );
        assert_eq!(report["complete"], true);
        if id == 2 {
            assert_eq!(report["stats"]["checked_functions"], 1);
            assert_eq!(report["stats"]["reused_functions"], 3);
        }
        if id == 3 {
            assert_eq!(report["stats"]["checked_functions"], 0);
            assert_eq!(report["stats"]["reused_functions"], 4);
        }
    }
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[test]
fn serve_streams_typed_reports_with_arbitrary_ids_and_unicode_on_one_line() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let id = serde_json::json!({"editor": "сессия\n1", "seq": [1, 2]});
    let request = serde_json::json!({
        "id": id,
        "files": [{
            "path": "é.clj",
            "source": "(defn label {:t/sig '[:-> string?]} [] \"été\")"
        }],
        "externs": null
    });
    writeln!(input, "{request}").unwrap();
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    let response: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], id);
    assert_eq!(response["result"]["version"], 1);
    assert_eq!(response["result"]["files"][0]["path"], "é.clj");
    assert_eq!(response["result"]["files"][0]["ok"], true);
    assert_eq!(
        response["result"]["files"][0]["diagnostics"],
        serde_json::json!([])
    );
    drop(input);
    assert!(child.wait().unwrap().success());
}
