//! Explicit client for an already running, loopback nREPL server.
mod bencode;

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::Args;
use serde::Serialize;

use bencode::{Decoder, Dict, Value, invalid};

const RECOVERY_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Args)]
pub(crate) struct Options {
    /// Port of an already running nREPL server on 127.0.0.1.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
    /// Limit each operation to this many milliseconds.
    #[arg(long, default_value_t = 5000, value_parser = clap::value_parser!(u64).range(1..=300_000))]
    timeout_ms: u64,
    /// Load this saved Clojure source file before evaluation. Repeat to load more files.
    #[arg(long = "load-file", value_name = "FILE")]
    files: Vec<PathBuf>,
    /// Evaluate this explicit expression. Repeat to reuse one session.
    #[arg(long = "eval", value_name = "CODE")]
    expressions: Vec<String>,
    /// Read Clojure source from stdin until EOF and evaluate it after any --eval forms.
    #[arg(long)]
    eval_stdin: bool,
}

#[derive(Default, Serialize)]
struct Report {
    ops: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    loads: Vec<Load>,
    evals: Vec<Eval>,
    session_closed: Option<bool>,
}

impl Report {
    fn failed(&self) -> bool {
        self.loads.iter().any(|load| load.result.failed()) || self.evals.iter().any(Eval::failed)
    }
}

#[derive(Serialize)]
struct Load {
    path: String,
    #[serde(flatten)]
    result: Eval,
}

#[derive(Default, Serialize)]
struct Eval {
    values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    namespace: Option<String>,
    out: String,
    err: String,
    exceptions: Vec<String>,
    root_exceptions: Vec<String>,
    status: Vec<String>,
    done: bool,
}

impl Eval {
    fn record(&mut self, message: &Dict, status: &[&str]) -> io::Result<()> {
        if let Some(value) = field_text(message, "value")? {
            self.values.push(value.to_owned());
        }
        if let Some(namespace) = field_text(message, "ns")?
            && !namespace.is_empty()
            && self.namespace.as_deref() != Some(namespace)
        {
            self.namespace = Some(namespace.to_owned());
        }
        if let Some(out) = field_text(message, "out")? {
            self.out.push_str(out);
        }
        if let Some(err) = field_text(message, "err")? {
            self.err.push_str(err);
        }
        if let Some(exception) = field_text(message, "ex")? {
            self.exceptions.push(exception.to_owned());
        }
        if let Some(exception) = field_text(message, "root-ex")? {
            self.root_exceptions.push(exception.to_owned());
        }
        self.status
            .extend(status.iter().map(|status| (*status).to_owned()));
        self.done = status.contains(&"done");
        Ok(())
    }

    fn failed(&self) -> bool {
        !self.exceptions.is_empty()
            || !self.root_exceptions.is_empty()
            || self.status.iter().any(|status| {
                matches!(
                    status.as_str(),
                    "error" | "eval-error" | "interrupted" | "namespace-not-found" | "no-code"
                )
            })
    }
}

pub(crate) fn run(options: Options) -> Result<u8, String> {
    let mut report = Report::default();
    let result = execute(options, &mut report);
    println!(
        "{}",
        serde_json::to_string(&report).map_err(|error| error.to_string())?
    );
    result.map(|()| u8::from(report.failed()))
}

fn execute(mut options: Options, report: &mut Report) -> Result<(), String> {
    if options.eval_stdin {
        let mut source = String::new();
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| format!("read nREPL source from stdin: {error}"))?;
        options.expressions.push(source);
    }
    let sources = options
        .files
        .iter()
        .map(|path| {
            let source = fs::read_to_string(path)
                .map_err(|error| format!("read nREPL source {}: {error}", path.display()))?;
            Ok((path, source))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut client = Client::connect(options.port, Duration::from_millis(options.timeout_ms))?;
    let mut described = false;
    client.request("describe", &[], |message, _| {
        if let Some(ops) = message.get("ops") {
            report.ops.extend(ops.dict()?.keys().cloned());
            described = true;
        }
        Ok(())
    })?;
    if !described {
        return Err("nREPL describe response did not include available operations".into());
    }
    report.ops.sort_unstable();
    report.ops.dedup();
    if sources.is_empty() && options.expressions.is_empty() {
        return Ok(());
    }

    for (required, needed) in [
        ("clone", true),
        ("load-file", !sources.is_empty()),
        ("eval", !options.expressions.is_empty()),
        ("close", true),
    ] {
        if needed && !report.ops.iter().any(|op| op == required) {
            return Err(format!("nREPL server does not advertise {required}"));
        }
    }

    let mut session = None;
    let cloned = client.request("clone", &[], |message, _| {
        if let Some(id) = field_text(message, "new-session")? {
            session = Some(id.to_owned());
        }
        Ok(())
    });
    let Some(session) = session.filter(|id| !id.is_empty()) else {
        cloned?;
        return Err("nREPL clone response did not include a session ID".into());
    };
    report.session_closed = Some(false);
    if let Err(error) = cloned {
        report.session_closed = Some(recover_session(options.port, &session, None));
        return Err(error.into());
    }

    if let Err(error) = run_session(&mut client, &session, sources, &options.expressions, report) {
        let interrupt_id = error
            .timed_out_id
            .as_deref()
            .filter(|_| report.ops.iter().any(|op| op == "interrupt"));
        report.session_closed = Some(recover_session(options.port, &session, interrupt_id));
        return Err(error.into());
    }
    let closed = close_session(&mut client, &session)?;
    report.session_closed = Some(closed);
    if !closed {
        return Err("nREPL server did not confirm session closure".into());
    }
    Ok(())
}

fn run_session(
    client: &mut Client,
    session: &str,
    sources: Vec<(&PathBuf, String)>,
    expressions: &[String],
    report: &mut Report,
) -> Result<(), RequestError> {
    for (path, source) in sources {
        let path_text = path.to_string_lossy();
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let mut result = Eval::default();
        let exchange = client.request(
            "load-file",
            &[
                ("session", session),
                ("file", &source),
                ("file-name", &name),
                ("file-path", &path_text),
            ],
            |message, status| result.record(message, status),
        );
        let failed = result.failed();
        report.loads.push(Load {
            path: path_text.into_owned(),
            result,
        });
        exchange?;
        if failed {
            break;
        }
    }
    if !report.failed() {
        evaluate(client, session, expressions, &mut report.evals)?;
    }
    Ok(())
}

fn close_session(client: &mut Client, session: &str) -> Result<bool, RequestError> {
    let mut closed = false;
    client.request("close", &[("session", session)], |_, status| {
        closed |= status.contains(&"session-closed");
        Ok(())
    })?;
    Ok(closed)
}

fn recover_session(port: u16, session: &str, interrupt_id: Option<&str>) -> bool {
    // A failed decoder may have consumed the beginning of a frame. Keep the
    // original socket alive for runtime output and use a fresh control stream.
    let Ok(mut control) = Client::connect(port, RECOVERY_TIMEOUT) else {
        return false;
    };
    if let Some(id) = interrupt_id
        && control
            .request(
                "interrupt",
                &[("session", session), ("interrupt-id", id)],
                |_, _| Ok(()),
            )
            .is_err()
    {
        let Ok(fresh) = Client::connect(port, RECOVERY_TIMEOUT) else {
            return false;
        };
        control = fresh;
    }
    close_session(&mut control, session).unwrap_or(false)
}

fn evaluate(
    client: &mut Client,
    session: &str,
    expressions: &[String],
    evals: &mut Vec<Eval>,
) -> Result<(), RequestError> {
    for code in expressions {
        evals.push(Eval::default());
        let eval = evals.last_mut().unwrap();
        client.request(
            "eval",
            &[("session", session), ("code", code)],
            |message, status| eval.record(message, status),
        )?;
        if eval.failed() {
            break;
        }
    }
    Ok(())
}

struct RequestError {
    message: String,
    timed_out_id: Option<String>,
}

impl From<RequestError> for String {
    fn from(error: RequestError) -> Self {
        error.message
    }
}

struct Client {
    socket: BufReader<TcpStream>,
    timeout: Duration,
    next_id: usize,
}

impl Client {
    fn connect(port: u16, timeout: Duration) -> Result<Self, String> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let socket = TcpStream::connect_timeout(&address, timeout)
            .map_err(|error| format!("nREPL connect to {address}: {error}"))?;
        Ok(Self {
            socket: BufReader::new(socket),
            timeout,
            next_id: 1,
        })
    }

    fn request(
        &mut self,
        op: &str,
        args: &[(&str, &str)],
        mut on_message: impl FnMut(&Dict, &[&str]) -> io::Result<()>,
    ) -> Result<(), RequestError> {
        let id = self.next_id.to_string();
        self.next_id += 1;
        let mut fields = BTreeMap::from_iter(args.iter().copied());
        fields.insert("op", op);
        fields.insert("id", &id);
        let request = bencode::encode(&fields);
        let timeout = self.timeout;
        let mut exchange = || -> io::Result<()> {
            let deadline = Instant::now() + timeout;
            let mut outgoing = request.as_slice();
            let mut stream = self.socket.get_ref();
            while !outgoing.is_empty() {
                stream.set_write_timeout(Some(time_left(deadline)?))?;
                let n = stream.write(outgoing)?;
                if n == 0 {
                    return Err(io::Error::from(io::ErrorKind::WriteZero));
                }
                outgoing = &outgoing[n..];
            }

            let mut decoder = Decoder::new(TimedInput {
                socket: &mut self.socket,
                deadline,
            });
            loop {
                let message = decoder.message()?;
                // Do not conflate unsolicited output or another request's frames.
                if field_text(&message, "id")? != Some(id.as_str()) {
                    continue;
                }
                let status = statuses(&message)?;
                on_message(&message, &status)?;
                if let Some(rejection) = status.iter().find(|status| {
                    matches!(
                        **status,
                        "unknown-op" | "unknown-session" | "session-closed"
                    ) && !(op == "close" && **status == "session-closed")
                }) {
                    return Err(invalid(format!("server rejected the request: {rejection}")));
                }
                if status.contains(&"done") {
                    return Ok(());
                }
            }
        };
        exchange().map_err(|error| {
            let timed_out = matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            );
            let message = match error.kind() {
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => format!(
                    "nREPL {op} timed out before done after {} ms",
                    timeout.as_millis()
                ),
                io::ErrorKind::UnexpectedEof
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::BrokenPipe => format!("nREPL {op}: connection closed before done"),
                _ => format!("nREPL {op}: {error}"),
            };
            RequestError {
                message,
                timed_out_id: timed_out.then_some(id),
            }
        })
    }
}

fn field_text<'a>(message: &'a Dict, field: &str) -> io::Result<Option<&'a str>> {
    message.get(field).map(Value::text).transpose()
}

fn statuses(message: &Dict) -> io::Result<Vec<&str>> {
    match message.get("status") {
        Some(value) => value.list()?.iter().map(Value::text).collect(),
        None => Ok(Vec::new()),
    }
}

struct TimedInput<'a> {
    socket: &'a mut BufReader<TcpStream>,
    deadline: Instant,
}

impl Read for TimedInput<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let timeout = time_left(self.deadline)?;
        if self.socket.buffer().is_empty() {
            self.socket.get_ref().set_read_timeout(Some(timeout))?;
        }
        self.socket.read(output)
    }
}

fn time_left(deadline: Instant) -> io::Result<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(io::Error::from(io::ErrorKind::TimedOut));
    }
    Ok(left)
}
