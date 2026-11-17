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
    assert!(
        text.contains("Argument 1: expected Number, got String"),
        "{text}"
    );
    assert!(text.contains("this expression has type `String`"), "{text}");
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
            assert!(
                text.contains("31m"),
                "Expected red error highlighting: {text}"
            );
        }
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
