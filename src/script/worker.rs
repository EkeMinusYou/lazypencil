use super::{MAX_OUTPUT_BYTES, Reply, ScriptOutput, ScriptRequest, invalid, validate_output};
use crate::config::runtime::ScriptSettings;
use crate::{Error, Result};
use rquickjs::{CatchResultExt, Context, Function, Runtime, context::EvalOptions};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
const WORKER_PROTOCOL: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<T> {
    protocol: u32,
    limits: ScriptSettings,
    request: T,
}

/// A process is reused across requests; every request gets a fresh JS runtime.
pub struct ScriptRuntime {
    executable: PathBuf,
    process: Option<Process>,
    cancel: Option<tokio_util::sync::CancellationToken>,
    settings: ScriptSettings,
}

struct Process {
    child: Child,
    last_used: Instant,
    input: Option<ChildStdin>,
    output: Option<BufReader<ChildStdout>>,
    _directory: tempfile::TempDir,
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl ScriptRuntime {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            process: None,
            cancel: None,
            settings: Default::default(),
        }
    }
    pub fn set_cancellation(&mut self, cancel: Option<tokio_util::sync::CancellationToken>) {
        self.cancel = cancel;
    }
    pub fn configure(&mut self, settings: ScriptSettings) -> Result<()> {
        settings.validate()?;
        self.settings = settings;
        self.expire_idle(Instant::now());
        Ok(())
    }
    pub fn settings(&self) -> ScriptSettings {
        self.settings
    }
    #[cfg(feature = "rendering")]
    pub(crate) fn cancellation(&self) -> Option<tokio_util::sync::CancellationToken> {
        self.cancel.clone()
    }
    pub fn current() -> Result<Self> {
        let mut runtime = Self::new(std::env::current_exe().map_err(|e| {
            Error::new(
                "RUNTIME_UNAVAILABLE",
                format!("Cannot locate script worker executable: {e}"),
            )
        })?);
        runtime.set_cancellation(Some(crate::cancel::token()));
        Ok(runtime)
    }
    pub fn execute(&mut self, request: &ScriptRequest) -> Result<ScriptOutput> {
        self.execute_until(request, None)
    }
    /// Bound startup, I/O and execution by an additional absolute host deadline.
    pub fn execute_until(
        &mut self,
        request: &ScriptRequest,
        deadline: Option<Instant>,
    ) -> Result<ScriptOutput> {
        self.expire_idle(Instant::now());
        self.execute_request(request, deadline)
    }
    /// The owned child ID, for runtime diagnostics.
    pub fn process_id(&self) -> Option<u32> {
        self.process.as_ref().map(|process| process.child.id())
    }
    /// Hosts should wake at this deadline even when no document request arrives.
    pub fn idle_deadline(&self) -> Option<Instant> {
        self.process
            .as_ref()
            .map(|process| process.last_used + Duration::from_secs(self.settings.idle_seconds))
    }
    /// Reap an idle child while preserving the runtime's ability to start another.
    pub fn expire_idle(&mut self, now: Instant) {
        if self.idle_deadline().is_some_and(|deadline| deadline <= now) {
            self.process.take();
        }
    }
    fn execute_request(
        &mut self,
        request: &ScriptRequest,
        deadline: Option<Instant>,
    ) -> Result<ScriptOutput> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|cancel| cancel.is_cancelled())
        {
            return Err(Error::new("CANCELLED", "Script request cancelled"));
        }
        super::header::Header::parse(&request.code)?;
        let started = Instant::now();
        let mut limits = self.settings;
        if let Some(deadline) = deadline {
            let remaining = deadline
                .checked_duration_since(started)
                .filter(|remaining| !remaining.is_zero())
                .ok_or_else(|| {
                    Error::new(
                        "RENDER_TIMEOUT",
                        "Script layout exceeded its configured deadline",
                    )
                })?;
            let remaining_ms = u64::try_from(remaining.as_millis())
                .unwrap_or(u64::MAX)
                .max(1);
            limits.timeout_ms = limits.timeout_ms.min(remaining_ms);
        }
        let watchdog = Duration::from_millis(limits.timeout_ms + 1000);
        let mut bytes = serde_json::to_vec(&Envelope {
            protocol: WORKER_PROTOCOL,
            limits,
            request,
        })
        .map_err(|e| invalid(format!("Cannot encode script input: {e}")))?;
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(invalid("Script request exceeds 4 MiB"));
        }
        bytes.push(b'\n');
        if started.elapsed() >= watchdog
            || deadline.is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(Error::new(
                "RENDER_TIMEOUT",
                "Script worker exceeded its configured deadline",
            ));
        }
        if self.process.is_none() {
            self.process = Some(start(&self.executable)?);
        }
        let process = self.process.as_mut().expect("started worker");
        let mut input = process.input.take().expect("idle input");
        let mut output = process.output.take().expect("idle output");
        let (sender, receiver) = mpsc::sync_channel(1);
        let io = std::thread::Builder::new()
            .name("lazypencil-script-io".into())
            .spawn(move || {
                let read = (|| -> std::io::Result<Vec<u8>> {
                    input.write_all(&bytes)?;
                    input.flush()?;
                    let mut response = Vec::new();
                    output
                        .by_ref()
                        .take(MAX_OUTPUT_BYTES as u64 + 1)
                        .read_until(b'\n', &mut response)?;
                    Ok(response)
                })();
                let _ = sender.send((read, input, output));
            });
        let io = match io {
            Ok(io) => io,
            Err(error) => {
                self.process.take();
                return Err(invalid(format!("Cannot start worker I/O: {error}")));
            }
        };
        let received = loop {
            if self
                .cancel
                .as_ref()
                .is_some_and(|cancel| cancel.is_cancelled())
            {
                self.process.take();
                let _ = io.join();
                return Err(Error::new(
                    "CANCELLED",
                    "Script worker cancelled and terminated",
                ));
            }
            if started.elapsed() >= watchdog
                || deadline.is_some_and(|deadline| Instant::now() >= deadline)
            {
                break Err(());
            }
            let wait = deadline.map_or(Duration::from_millis(20), |deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(20))
            });
            match receiver.recv_timeout(wait) {
                Ok(response) => break Ok(response),
                Err(mpsc::RecvTimeoutError::Disconnected) => break Err(()),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
            }
        };
        let response = match received {
            Ok((read, input, output)) => {
                process.input = Some(input);
                process.output = Some(output);
                let _ = io.join();
                process.last_used = Instant::now();
                read
            }
            Err(_) => {
                self.process.take();
                let _ = io.join();
                return Err(Error::new(
                    "RENDER_TIMEOUT",
                    "Script worker exceeded its deadline and was terminated",
                ));
            }
        };
        let response = match response {
            Ok(bytes)
                if !bytes.is_empty()
                    && bytes.len() <= MAX_OUTPUT_BYTES
                    && bytes.last() == Some(&b'\n') =>
            {
                bytes
            }
            _ => {
                self.process.take();
                return Err(invalid(
                    "Script worker stopped or returned an incomplete or oversized reply",
                ));
            }
        };
        if started.elapsed() >= watchdog
            || deadline.is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.process.take();
            return Err(Error::new(
                "RENDER_TIMEOUT",
                "Script worker exceeded its configured deadline",
            ));
        }
        let reply: Reply = serde_json::from_slice(&response).map_err(|_| {
            self.process.take();
            invalid("Script worker returned an invalid reply")
        })?;
        match reply {
            Reply::Success { output } => {
                validate_output(&output.nodes)?;
                if started.elapsed() >= watchdog
                    || deadline.is_some_and(|deadline| Instant::now() >= deadline)
                {
                    self.process.take();
                    return Err(Error::new(
                        "RENDER_TIMEOUT",
                        "Script worker exceeded its configured deadline",
                    ));
                }
                Ok(output)
            }
            Reply::Failure {
                message,
                timeout,
                details,
            } => {
                let mut error = if timeout {
                    self.process.take();
                    Error::new("RENDER_TIMEOUT", message)
                } else {
                    invalid(message)
                };
                error.details = details;
                Err(error)
            }
        }
    }
}

fn start(executable: &Path) -> Result<Process> {
    let executable = crate::paths::canonical(executable)?;
    let directory = tempfile::tempdir().map_err(|e| {
        Error::new(
            "RUNTIME_UNAVAILABLE",
            format!("Cannot create script worker directory: {e}"),
        )
    })?;
    let mut child = Command::new(executable)
        .arg("__script_worker")
        .env_clear()
        .current_dir(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            Error::new(
                "RUNTIME_UNAVAILABLE",
                format!("Cannot start script worker: {e}"),
            )
        })?;
    Ok(Process {
        input: child.stdin.take(),
        output: child.stdout.take().map(BufReader::new),
        child,
        last_used: Instant::now(),
        _directory: directory,
    })
}

pub(super) fn serve() -> Result<()> {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    loop {
        let mut bytes = Vec::new();
        let count = input
            .by_ref()
            .take(MAX_REQUEST_BYTES as u64 + 1)
            .read_until(b'\n', &mut bytes)
            .map_err(|e| invalid(format!("Cannot read script request: {e}")))?;
        if count == 0 {
            return Ok(());
        }
        if bytes.len() > MAX_REQUEST_BYTES || bytes.last() != Some(&b'\n') {
            return Err(invalid("Script request exceeds its framing limit"));
        }
        let result = serde_json::from_slice::<Envelope<ScriptRequest>>(&bytes)
            .map_err(|_| invalid("Invalid script request"))
            .and_then(|envelope| {
                if envelope.protocol != WORKER_PROTOCOL {
                    return Err(invalid("Unsupported script worker protocol"));
                }
                envelope
                    .limits
                    .validate()
                    .map_err(|_| invalid("Invalid script worker limits"))?;
                execute(envelope.request, envelope.limits)
            });
        let reply = match result {
            Ok(output) => Reply::Success { output },
            Err(error) => Reply::Failure {
                message: error.message,
                timeout: error.code == "RENDER_TIMEOUT",
                details: error.details,
            },
        };
        let mut bytes =
            serde_json::to_vec(&reply).map_err(|_| invalid("Cannot encode script reply"))?;
        if bytes.len() >= MAX_OUTPUT_BYTES {
            bytes = serde_json::to_vec(&Reply::Failure {
                message: "Script output exceeds 4 MiB".into(),
                timeout: false,
                details: None,
            })
            .expect("bounded reply");
        }
        output
            .write_all(&bytes)
            .and_then(|_| output.write_all(b"\n"))
            .and_then(|_| output.flush())
            .map_err(|e| invalid(format!("Cannot write script reply: {e}")))?;
    }
}

fn execute(request: ScriptRequest, limits: ScriptSettings) -> Result<ScriptOutput> {
    if [request.width, request.height]
        .iter()
        .any(|n| !n.is_finite() || *n < 0.0 || *n > 1_000_000.0)
    {
        return Err(invalid(
            "Script dimensions must be finite and within 0..1000000",
        ));
    }
    let header = super::header::Header::parse(&request.code)?;
    let inputs = header.prepare(&request.inputs)?;
    let started = Instant::now();
    let runtime =
        Runtime::new().map_err(|e| invalid(format!("Cannot initialize JavaScript: {e}")))?;
    runtime.set_memory_limit(limits.heap_mib as usize * 1024 * 1024);
    runtime.set_max_stack_size(512 * 1024);
    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = interrupted.clone();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        let stop = started.elapsed() >= Duration::from_millis(limits.timeout_ms);
        if stop {
            flag.store(true, Ordering::Relaxed);
        }
        stop
    })));
    let context = Context::full(&runtime)
        .map_err(|e| invalid(format!("Cannot create JavaScript context: {e}")))?;
    let serialized = context.with(|ctx| -> Result<String> {
        ctx.globals()
            .set(
                "__script_inputs",
                serde_json::to_string(&inputs).expect("finite JSON"),
            )
            .map_err(js_error)?;
        ctx.globals()
            .set("__script_width", request.width)
            .map_err(js_error)?;
        ctx.globals()
            .set("__script_height", request.height)
            .map_err(js_error)?;
        // Capture normalization APIs and remove temporary globals before source
        // evaluation, as in the original Function-constructor execution path.
        let run = ctx
            .eval_with_options::<Function, _>(
                include_str!("sandbox.js"),
                eval_options("lazypencil-sandbox.js"),
            )
            .catch(&ctx)
            .map_err(|error| super::diagnostic::javascript(error, &request.code, "setup"))?;
        // Match the pinned engine's Function("pencil", strict + source) wrapper
        // in the global environment, while giving user frames a distinct name.
        // The body cannot capture the sandbox's private lexical environment.
        let function = ctx
            .eval_with_options::<Function, _>(
                format!(
                    "(function anonymous(pencil\n) {{\n\"use strict\";\n{}\n}})",
                    request.code
                ),
                eval_options(super::diagnostic::SOURCE_NAME),
            )
            .catch(&ctx)
            .map_err(|error| super::diagnostic::javascript(error, &request.code, "compile"))?;
        run.call::<_, String>((function,))
            .catch(&ctx)
            .map_err(|error| super::diagnostic::javascript(error, &request.code, "runtime"))
    });
    if interrupted.load(Ordering::Relaxed)
        || started.elapsed() >= Duration::from_millis(limits.timeout_ms)
    {
        let mut error = serialized
            .err()
            .unwrap_or_else(|| invalid("Script execution exceeded its configured deadline"));
        error.code = "RENDER_TIMEOUT".into();
        return Err(error);
    }
    let serialized = serialized?;
    if runtime.is_job_pending() {
        return Err(invalid(
            "Scripts must run synchronously; queued promise jobs are not supported",
        ));
    }
    if serialized.len() > MAX_OUTPUT_BYTES {
        return Err(invalid("Script output exceeds 4 MiB"));
    }
    let output: ScriptOutput = serde_json::from_str(&serialized)
        .map_err(|_| invalid("Script output must be finite JSON node data"))?;
    validate_output(&output.nodes)?;
    Ok(output)
}

fn js_error(error: rquickjs::Error) -> Error {
    invalid(format!("JavaScript setup failed: {error}"))
}

fn eval_options(filename: &str) -> EvalOptions {
    let mut options = EvalOptions::default();
    options.filename = Some(filename.into());
    options
}
