use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const DESCRIBE: &[u8] = b"d2:id1:12:op8:describee";
const DESCRIBED: &[u8] = b"d2:id1:13:opsd5:closede5:clonede8:describede4:evaldee6:statusl4:doneee";
const DESCRIBED_WITH_LOAD: &[u8] =
    b"d2:id1:13:opsd5:closede5:clonede8:describede4:evalde9:load-filedee6:statusl4:doneee";
const DESCRIBED_WITH_INTERRUPT: &[u8] =
    b"d2:id1:13:opsd5:closede5:clonede8:describede4:evalde9:interruptdee6:statusl4:doneee";
const CLONE: &[u8] = b"d2:id1:22:op5:clonee";
const CLONED: &[u8] = b"d2:id1:211:new-session4:test6:statusl4:doneee";
const EVAL: &[u8] = b"d4:code8:(+ 40 2)2:id1:32:op4:eval7:session4:teste";
const INTERRUPT: &[u8] = b"d2:id1:112:interrupt-id1:32:op9:interrupt7:session4:teste";
const INTERRUPTED_EVAL: &[u8] = b"d2:id1:36:statusl4:done11:interruptedee";
const CLOSE_ON_FRESH_CONNECTION: &[u8] = b"d2:id1:12:op5:close7:session4:teste";
const CLOSED_ON_FRESH_CONNECTION: &[u8] = b"d2:id1:16:statusl4:done14:session-closedee";
const UNFINISHED_RESULT: &[u8] = concat!(
    "d2:id1:32:ns4:user5:value2:42e",
    "d2:id1:33:out7:partiale",
    "d3:err7:warning2:id1:3e",
    "d2:id1:33:out16:truncated",
)
.as_bytes();
const LOAD_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/nrepl/load.clj");

fn fixture(args: &[&str], serve: impl FnOnce(TcpStream) + Send + 'static) -> Output {
    fixture_with_input(args, "", serve)
}

fn fixture_with_input(
    args: &[&str],
    input: &str,
    serve: impl FnOnce(TcpStream) + Send + 'static,
) -> Output {
    fixture_with_listener(args, input, move |listener| serve(accept(&listener)))
}

fn fixture_with_listener(
    args: &[&str],
    input: &str,
    serve: impl FnOnce(TcpListener) + Send + 'static,
) -> Output {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback test server");
    let port = listener.local_addr().unwrap().port().to_string();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || serve(listener));
    let mut child = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args(["nrepl", "--port", &port])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    server.join().unwrap();
    output
}

fn accept(listener: &TcpListener) -> TcpStream {
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                let limit = Some(Duration::from_secs(3));
                stream.set_read_timeout(limit).unwrap();
                stream.set_write_timeout(limit).unwrap();
                return stream;
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    && start.elapsed() < Duration::from_secs(3) =>
            {
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("could not accept client connection: {error}"),
        }
    }
}

fn exchange(stream: &mut TcpStream, request: &[u8], response: &[u8]) {
    let mut bytes = vec![0; request.len()];
    stream.read_exact(&mut bytes).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&bytes),
        String::from_utf8_lossy(request)
    );
    stream.write_all(response).unwrap();
}

fn open(stream: &mut TcpStream) {
    exchange(stream, DESCRIBE, DESCRIBED);
    exchange(stream, CLONE, CLONED);
}

fn load_request() -> String {
    let source = include_str!("fixtures/nrepl/load.clj");
    format!(
        "d4:file{}:{source}9:file-name8:load.clj9:file-path{}:{LOAD_FILE}2:id1:32:op9:load-file7:session4:teste",
        source.len(),
        LOAD_FILE.len(),
    )
}

fn assert_disconnected(stream: &mut TcpStream) {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    assert!(bytes.is_empty(), "{bytes:?}");
}

fn assert_open_without_request(stream: &TcpStream) {
    let original_timeout = stream.read_timeout().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(25)))
        .unwrap();
    let peek = stream.peek(&mut [0]);
    stream.set_read_timeout(original_timeout).unwrap();
    assert!(
        matches!(peek, Err(ref error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)),
        "expected an open socket with no pending request, got {peek:?}"
    );
}

fn report(output: &Output, exit: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(exit),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn describes_without_creating_a_session() {
    let output = fixture(&[], |mut stream| {
        exchange(&mut stream, DESCRIBE, DESCRIBED);
        assert_disconnected(&mut stream);
    });
    assert_eq!(
        report(&output, 0),
        json!({"ops":["clone","close","describe","eval"],"evals":[],"session_closed":null})
    );
}

#[test]
fn reuses_a_session_and_closes_it_after_an_eval_error() {
    let output = fixture(
        &["--eval", "(+ 40 2)", "--eval", "bad", "--eval", "never"],
        |mut stream| {
            open(&mut stream);
            exchange(
                &mut stream,
                EVAL,
                // The done from another request must not end this one.
                b"d2:id1:96:statusl4:doneeed2:id1:33:out5:helloed3:err4:w",
            );
            stream
                .write_all(b"arn2:id1:35:value2:42ed2:id1:36:statusl4:doneee")
                .unwrap();
            exchange(
                &mut stream,
                b"d4:code3:bad2:id1:42:op4:eval7:session4:teste",
                b"d2:ex4:Oops2:id1:46:statusl10:eval-erroreed2:id1:46:statusl4:doneee",
            );
            exchange(
                &mut stream,
                b"d2:id1:52:op5:close7:session4:teste",
                b"d2:id1:56:statusl4:done14:session-closedee",
            );
        },
    );
    let report = report(&output, 1);
    assert_eq!(report["session_closed"], true);
    assert_eq!(
        report["evals"],
        json!([
            {
                "values": ["42"], "out": "hello", "err": "warn",
                "exceptions": [], "root_exceptions": [], "status": ["done"], "done": true
            },
            {
                "values": [], "out": "", "err": "",
                "exceptions": ["Oops"], "root_exceptions": [],
                "status": ["eval-error", "done"], "done": true
            }
        ])
    );
}

#[test]
fn loads_saved_source_before_eval_in_the_same_session() {
    let expression = "(+ (parenox-nrepl-fixture/score) 2)";
    let output = fixture(
        &["--load-file", LOAD_FILE, "--eval", expression],
        move |mut stream| {
            exchange(&mut stream, DESCRIBE, DESCRIBED_WITH_LOAD);
            exchange(&mut stream, CLONE, CLONED);
            exchange(
                &mut stream,
                load_request().as_bytes(),
                "d2:id1:33:out10:loaded λ\ned3:err13:load warning\n2:id1:3ed2:id1:32:ns21:parenox-nrepl-fixture5:value7::loadeded2:id1:36:statusl4:doneee".as_bytes(),
            );
            let eval = format!(
                "d4:code{}:{expression}2:id1:42:op4:eval7:session4:teste",
                expression.len()
            );
            exchange(
                &mut stream,
                eval.as_bytes(),
                b"d2:id1:42:ns4:usered2:id1:42:ns21:parenox-nrepl-fixture5:value2:42ed2:id1:46:statusl4:doneee",
            );
            exchange(
                &mut stream,
                b"d2:id1:52:op5:close7:session4:teste",
                b"d2:id1:56:statusl4:done14:session-closedee",
            );
        },
    );
    let report = report(&output, 0);
    assert_eq!(report["session_closed"], true);
    assert_eq!(
        report["loads"],
        json!([{
            "path": LOAD_FILE, "values": [":loaded"], "namespace": "parenox-nrepl-fixture",
            "out": "loaded λ\n", "err": "load warning\n",
            "exceptions": [], "root_exceptions": [], "status": ["done"], "done": true
        }])
    );
    assert_eq!(report["evals"][0]["values"], json!(["42"]));
    assert_eq!(report["evals"][0]["namespace"], "parenox-nrepl-fixture");
}

#[test]
fn closes_the_session_after_a_load_error_without_evaluating() {
    let output = fixture(
        &["--load-file", LOAD_FILE, "--eval", "never"],
        |mut stream| {
            exchange(&mut stream, DESCRIBE, DESCRIBED_WITH_LOAD);
            exchange(&mut stream, CLONE, CLONED);
            exchange(
                &mut stream,
                load_request().as_bytes(),
                b"d2:ex4:Oops2:id1:36:statusl10:eval-error4:doneee",
            );
            exchange(
                &mut stream,
                b"d2:id1:42:op5:close7:session4:teste",
                b"d2:id1:46:statusl4:done14:session-closedee",
            );
        },
    );
    let report = report(&output, 1);
    assert_eq!(report["session_closed"], true);
    assert_eq!(report["loads"][0]["exceptions"], json!(["Oops"]));
    assert_eq!(report["evals"], json!([]));
}

#[test]
fn sends_the_stdin_selection_verbatim_after_command_line_evals() {
    let selection = "\n(do\n  (println \"λ\")\n  (* 2 3))\n";
    let output = fixture_with_input(
        &["--eval-stdin", "--eval", "(+ 40 2)"],
        selection,
        move |mut stream| {
            open(&mut stream);
            exchange(
                &mut stream,
                EVAL,
                b"d2:id1:35:value2:42ed2:id1:36:statusl4:doneee",
            );
            let request = format!(
                "d4:code{}:{selection}2:id1:42:op4:eval7:session4:teste",
                selection.len()
            );
            exchange(
                &mut stream,
                request.as_bytes(),
                "d2:id1:43:out3:λ\ned2:id1:45:value1:6ed2:id1:46:statusl4:doneee".as_bytes(),
            );
            exchange(
                &mut stream,
                b"d2:id1:52:op5:close7:session4:teste",
                b"d2:id1:56:statusl4:done14:session-closedee",
            );
        },
    );
    let report = report(&output, 0);
    assert_eq!(report["session_closed"], true);
    assert_eq!(report["evals"][0]["values"], json!(["42"]));
    assert_eq!(report["evals"][1]["values"], json!(["6"]));
    assert_eq!(report["evals"][1]["out"], "λ\n");
}

#[test]
fn rejects_missing_eval_before_opening_a_session() {
    let output = fixture(&["--eval", "(+ 40 2)"], |mut stream| {
        exchange(
            &mut stream,
            DESCRIBE,
            b"d2:id1:13:opsd5:closede5:clonede8:describedee6:statusl4:doneee",
        );
        assert_disconnected(&mut stream);
    });
    assert_eq!(report(&output, 2)["session_closed"], Value::Null);
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not advertise eval"));
}

#[test]
fn times_out_without_confusing_partial_output_with_completion() {
    let output = fixture_with_listener(
        &["--timeout-ms", "150", "--eval", "(+ 40 2)"],
        "",
        |listener| {
            let mut stream = accept(&listener);
            drop(listener);
            open(&mut stream);
            exchange(&mut stream, EVAL, b"d2:id1:33:out7:partiale");
            assert_disconnected(&mut stream);
        },
    );
    let report = report(&output, 2);
    assert_eq!(report["session_closed"], false);
    assert_eq!(report["evals"][0]["out"], "partial");
    assert_eq!(report["evals"][0]["done"], false);
    assert!(String::from_utf8_lossy(&output.stderr).contains("eval timed out before done"));
}

#[test]
fn interrupts_and_closes_a_timed_out_eval_on_a_fresh_connection() {
    let output = fixture_with_listener(
        &[
            "--timeout-ms",
            "500",
            "--eval",
            "(+ 40 2)",
            "--eval",
            "never",
        ],
        "",
        |listener| {
            let mut original = accept(&listener);
            exchange(&mut original, DESCRIBE, DESCRIBED_WITH_INTERRUPT);
            exchange(&mut original, CLONE, CLONED);
            exchange(&mut original, EVAL, UNFINISHED_RESULT);

            let mut cleanup = accept(&listener);
            assert_open_without_request(&original);
            // Stock JVM nREPL sends the original eval's completion here first.
            exchange(&mut cleanup, INTERRUPT, INTERRUPTED_EVAL);
            assert_open_without_request(&cleanup);
            cleanup.write_all(b"d2:id1:16:statusl4:doneee").unwrap();
            exchange(
                &mut cleanup,
                b"d2:id1:22:op5:close7:session4:teste",
                b"d2:id1:26:statusl4:done14:session-closedee",
            );
            assert_disconnected(&mut original);
        },
    );
    let report = timed_out_report(&output, "eval");
    assert_eq!(report["session_closed"], true);
    assert_eq!(report["evals"], json!([unfinished_result()]));
}

#[test]
fn closes_a_timed_out_load_without_advertised_interrupt() {
    let output = fixture_with_listener(
        &[
            "--timeout-ms",
            "500",
            "--load-file",
            LOAD_FILE,
            "--eval",
            "never",
        ],
        "",
        |listener| {
            let mut original = accept(&listener);
            exchange(&mut original, DESCRIBE, DESCRIBED_WITH_LOAD);
            exchange(&mut original, CLONE, CLONED);
            exchange(&mut original, load_request().as_bytes(), UNFINISHED_RESULT);

            let mut cleanup = accept(&listener);
            assert_open_without_request(&original);
            exchange(
                &mut cleanup,
                CLOSE_ON_FRESH_CONNECTION,
                CLOSED_ON_FRESH_CONNECTION,
            );
            assert_disconnected(&mut original);
        },
    );
    let report = timed_out_report(&output, "load-file");
    assert_eq!(report["session_closed"], true);
    assert_eq!(report["evals"], json!([]));
    let mut load = unfinished_result();
    load["path"] = json!(LOAD_FILE);
    assert_eq!(report["loads"], json!([load]));
}

#[test]
fn reconnects_to_close_after_an_interrupt_transport_error() {
    let output = fixture_with_listener(
        &["--timeout-ms", "500", "--eval", "(+ 40 2)"],
        "",
        |listener| {
            let mut original = accept(&listener);
            exchange(&mut original, DESCRIBE, DESCRIBED_WITH_INTERRUPT);
            exchange(&mut original, CLONE, CLONED);
            exchange(&mut original, EVAL, UNFINISHED_RESULT);

            let mut interrupt = accept(&listener);
            exchange(&mut interrupt, INTERRUPT, b"d2:id1:16:statusl4:do");
            drop(interrupt);

            let mut cleanup = accept(&listener);
            assert_open_without_request(&original);
            exchange(
                &mut cleanup,
                CLOSE_ON_FRESH_CONNECTION,
                CLOSED_ON_FRESH_CONNECTION,
            );
            assert_disconnected(&mut original);
        },
    );
    let report = timed_out_report(&output, "eval");
    assert_eq!(report["session_closed"], true);
    assert_eq!(report["evals"], json!([unfinished_result()]));
}

#[test]
fn closes_a_session_reported_before_clone_times_out() {
    let output = fixture_with_listener(
        &["--timeout-ms", "500", "--eval", "never"],
        "",
        |listener| {
            let mut original = accept(&listener);
            exchange(&mut original, DESCRIBE, DESCRIBED_WITH_INTERRUPT);
            exchange(&mut original, CLONE, b"d2:id1:211:new-session4:teste");

            let mut cleanup = accept(&listener);
            assert_open_without_request(&original);
            exchange(
                &mut cleanup,
                CLOSE_ON_FRESH_CONNECTION,
                CLOSED_ON_FRESH_CONNECTION,
            );
            assert_disconnected(&mut original);
        },
    );
    let report = timed_out_report(&output, "clone");
    assert_eq!(report["session_closed"], true);
    assert_eq!(report["evals"], json!([]));
}

fn timed_out_report(output: &Output, op: &str) -> Value {
    let result = report(output, 2);
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!("parenox: nREPL {op} timed out before done after 500 ms\n")
    );
    result
}

fn unfinished_result() -> Value {
    json!({
        "values": ["42"], "namespace": "user", "out": "partial", "err": "warning", "exceptions": [],
        "root_exceptions": [], "status": [], "done": false
    })
}

#[test]
fn reports_disconnect_before_done_and_keeps_the_partial_value() {
    let output = fixture(&["--eval", "(+ 40 2)"], |mut stream| {
        open(&mut stream);
        exchange(&mut stream, EVAL, b"d2:id1:35:value2:42e");
    });
    let report = report(&output, 2);
    assert_eq!(report["session_closed"], false);
    assert_eq!(report["evals"][0]["values"], json!(["42"]));
    assert_eq!(report["evals"][0]["done"], false);
    assert!(String::from_utf8_lossy(&output.stderr).contains("connection closed before done"));
}

#[test]
fn rejects_oversized_bencode_before_allocating_a_reply() {
    let output = fixture(&[], |mut stream| {
        exchange(&mut stream, DESCRIBE, b"d2:id1:13:ops99999999:");
    });
    assert_eq!(report(&output, 2)["session_closed"], Value::Null);
    assert!(String::from_utf8_lossy(&output.stderr).contains("response exceeds byte limit"));
}
