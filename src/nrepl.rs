//! Explicit client for an already running, loopback nREPL server.
mod bencode;

use std::collections::BTreeMap;
use std::io::{self, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use clap::Args;
use serde::Serialize;

use bencode::{Decoder, Dict, Value, invalid};

#[derive(Args)]
pub(crate) struct Options {
    /// Port of an already running nREPL server on 127.0.0.1.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
    /// Limit each operation to this many milliseconds.
    #[arg(long, default_value_t = 5000, value_parser = clap::value_parser!(u64).range(1..=300_000))]
    timeout_ms: u64,
    /// Evaluate this explicit expression. Repeat to reuse one session.
    /// Without --eval only describe the server.
    #[arg(long = "eval", value_name = "CODE")]
    expressions: Vec<String>,
}

#[derive(Default, Serialize)]
struct Report {
    ops: Vec<String>,
    evals: Vec<Eval>,
    session_closed: Option<bool>,
}

#[derive(Default, Serialize)]
struct Eval {
    values: Vec<String>,
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
    result.map(|()| u8::from(report.evals.iter().any(Eval::failed)))
}

fn execute(options: Options, report: &mut Report) -> Result<(), String> {
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
    if options.expressions.is_empty() {
        return Ok(());
    }

    for required in ["clone", "eval", "close"] {
        if !report.ops.iter().any(|op| op == required) {
            return Err(format!("nREPL server does not advertise {required}"));
        }
    }

    let mut session = None;
    client.request("clone", &[], |message, _| {
        if let Some(id) = field_text(message, "new-session")? {
            session = Some(id.to_owned());
        }
        Ok(())
    })?;
    let session = session
        .filter(|id| !id.is_empty())
        .ok_or("nREPL clone response did not include a session ID")?;
    report.session_closed = Some(false);
    for code in &options.expressions {
        report.evals.push(Eval::default());
        let eval = report.evals.last_mut().unwrap();
        client.request(
            "eval",
            &[("session", &session), ("code", code)],
            |message, status| eval.record(message, status),
        )?;
        if eval.failed() {
            break;
        }
    }
    let mut closed = false;
    client.request("close", &[("session", &session)], |_, status| {
        closed |= status.contains(&"session-closed");
        Ok(())
    })?;
    report.session_closed = Some(closed);
    if !closed {
        return Err("nREPL server did not confirm session closure".into());
    }
    Ok(())
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
    ) -> Result<(), String> {
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
        exchange().map_err(|error| match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
                format!(
                    "nREPL {op} timed out before done after {} ms",
                    timeout.as_millis()
                )
            }
            io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::BrokenPipe => {
                format!("nREPL {op}: connection closed before done")
            }
            _ => format!("nREPL {op}: {error}"),
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
