use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const DESCRIBE: &[u8] = b"d2:id1:12:op8:describee";
const DESCRIBED: &[u8] = b"d2:id1:13:opsd5:closede5:clonede8:describede4:evaldee6:statusl4:doneee";
const CLONE: &[u8] = b"d2:id1:22:op5:clonee";
const CLONED: &[u8] = b"d2:id1:211:new-session4:test6:statusl4:doneee";
const EVAL: &[u8] = b"d4:code8:(+ 40 2)2:id1:32:op4:eval7:session4:teste";

fn fixture(args: &[&str], serve: impl FnOnce(TcpStream) + Send + 'static) -> Output {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback test server");
    let port = listener.local_addr().unwrap().port().to_string();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let start = Instant::now();
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let limit = Some(Duration::from_secs(3));
                    stream.set_read_timeout(limit).unwrap();
                    stream.set_write_timeout(limit).unwrap();
                    serve(stream);
                    return;
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
    });
    let output = Command::new(env!("CARGO_BIN_EXE_parenox"))
        .args(["nrepl", "--port", &port])
        .args(args)
        .output()
        .unwrap();
    server.join().unwrap();
    output
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

fn assert_disconnected(stream: &mut TcpStream) {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    assert!(bytes.is_empty(), "{bytes:?}");
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
    let output = fixture(
        &["--timeout-ms", "150", "--eval", "(+ 40 2)"],
        |mut stream| {
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
