use std::io;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use parenox_runtime::{Phase, RunError};

pub(super) fn run(
    command: Command,
    operation: &str,
    timeout: Option<Duration>,
) -> Result<u8, String> {
    if let Some(timeout) = timeout {
        return match supervise(command, timeout).map_err(|error| error.to_string())? {
            ProcessExit::Completed(status) => Ok(exit_code(status)),
            ProcessExit::Cancelled(signal) => Ok(128 + signal),
            ProcessExit::TimedOut => {
                eprintln!(
                    "parenox: Clojure {operation} timed out after {} ms",
                    timeout.as_millis()
                );
                Ok(124)
            }
        };
    }
    let program = command.get_program().to_string_lossy().into_owned();
    direct(command).map_err(|error| format!("could not start {program}: {error}"))
}

pub(super) fn report(
    mut command: Command,
    operation: &str,
    timeout: Duration,
    redirect_stdout: bool,
) -> Result<ExitStatus, RunError> {
    if redirect_stdout {
        // Stream the program log without a relay pipe that can outlive the child.
        let log = stderr_output().map_err(|error| {
            RunError::new(
                Phase::Launch,
                format!("could not redirect {operation} output: {error}"),
            )
        })?;
        command.stdout(log);
    }
    match supervise(command, timeout)? {
        ProcessExit::Completed(status) => Ok(status),
        ProcessExit::Cancelled(signal) => Err(RunError::new(
            Phase::Execution,
            format!("Clojure {operation} cancelled by signal {signal}"),
        )
        .with_exit_code(128 + signal)),
        ProcessExit::TimedOut => Err(RunError::new(
            Phase::Execution,
            format!(
                "Clojure {operation} timed out after {} ms",
                timeout.as_millis()
            ),
        )
        .with_exit_code(124)),
    }
}

#[cfg(unix)]
fn stderr_output() -> io::Result<Stdio> {
    use std::os::fd::AsFd;

    io::stderr().as_fd().try_clone_to_owned().map(Stdio::from)
}

#[cfg(windows)]
fn stderr_output() -> io::Result<Stdio> {
    use std::os::windows::io::AsHandle;

    io::stderr()
        .as_handle()
        .try_clone_to_owned()
        .map(Stdio::from)
}

#[cfg(not(any(unix, windows)))]
fn stderr_output() -> io::Result<Stdio> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "stderr duplication is unavailable",
    ))
}

fn supervise(mut command: Command, timeout: Duration) -> Result<ProcessExit, RunError> {
    let signals = Signals::configure(&mut command).map_err(|error| {
        RunError::new(
            Phase::Launch,
            format!("could not configure cancellation: {error}"),
        )
    })?;
    let mut child = command.spawn().map_err(|error| {
        RunError::new(
            Phase::Launch,
            format!(
                "could not start {}: {error}",
                command.get_program().display()
            ),
        )
    })?;
    wait(&mut child, timeout, signals).map_err(|error| RunError::new(Phase::Execution, error))
}

#[cfg(unix)]
fn direct(mut command: Command) -> io::Result<u8> {
    use std::os::unix::process::CommandExt;
    Err(command.exec())
}

#[cfg(not(unix))]
fn direct(mut command: Command) -> io::Result<u8> {
    command.status().map(exit_code)
}

enum ProcessExit {
    Completed(ExitStatus),
    Cancelled(u8),
    TimedOut,
}

fn wait(child: &mut Child, timeout: Duration, signals: Signals) -> Result<ProcessExit, String> {
    let started = Instant::now();
    loop {
        if let Some(signal) = signals.cancelled() {
            signals.stop(child)?;
            return Ok(ProcessExit::Cancelled(signal));
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(ProcessExit::Completed(status)),
            Ok(None) => {}
            Err(error) => {
                signals.stop(child)?;
                return Err(format!("wait for Clojure CLI: {error}"));
            }
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            signals.stop(child)?;
            return Ok(ProcessExit::TimedOut);
        }
        thread::sleep(remaining.min(Duration::from_millis(20)));
    }
}

fn exit_code(status: ExitStatus) -> u8 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return (128 + signal) as u8;
        }
    }
    status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1)
}

struct Signals {
    #[cfg(unix)]
    separate_group: bool,
}

#[cfg(unix)]
mod unix {
    use std::io::IsTerminal;
    use std::os::unix::process::CommandExt;
    use std::sync::atomic::{AtomicI32, Ordering};

    use super::*;

    static INTERRUPTED: AtomicI32 = AtomicI32::new(0);

    extern "C" fn cancel(signal: libc::c_int) {
        INTERRUPTED.store(signal, Ordering::Relaxed);
    }

    impl Signals {
        pub(super) fn configure(command: &mut Command) -> io::Result<Self> {
            // Keep terminal input in the foreground process group.
            let separate_group = !io::stdin().is_terminal();
            if separate_group {
                command.process_group(0);
            }
            for signal in [libc::SIGINT, libc::SIGTERM] {
                // The handler only stores the signal number in a lock-free atomic.
                if unsafe { libc::signal(signal, cancel as *const () as libc::sighandler_t) }
                    == libc::SIG_ERR
                {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(Self { separate_group })
        }

        pub(super) fn cancelled(&self) -> Option<u8> {
            let signal = INTERRUPTED.load(Ordering::Relaxed);
            (signal != 0).then_some(signal as u8)
        }

        pub(super) fn stop(&self, child: &mut Child) -> Result<(), String> {
            let result = if self.separate_group {
                // A negative PID addresses the child's process group.
                if unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL) } == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            } else {
                child.kill()
            };
            reap(child, result)
        }
    }
}

#[cfg(not(unix))]
impl Signals {
    fn configure(_: &mut Command) -> io::Result<Self> {
        Ok(Self {})
    }

    fn cancelled(&self) -> Option<u8> {
        None
    }

    fn stop(&self, child: &mut Child) -> Result<(), String> {
        let result = child.kill();
        reap(child, result)
    }
}

fn reap(child: &mut Child, kill: io::Result<()>) -> Result<(), String> {
    if let Err(error) = kill {
        // The process may have exited between the poll and the signal.
        if child.try_wait().ok().flatten().is_none() {
            return Err(format!("terminate Clojure CLI: {error}"));
        }
    }
    child
        .wait()
        .map_err(|error| format!("reap Clojure CLI: {error}"))?;
    Ok(())
}
