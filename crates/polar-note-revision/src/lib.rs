//! Runs the bundled note revision helper with bounded, cancellable local I/O.

use std::{
    error, fmt,
    io::{self, Read, Write},
    ops::Range,
    path::Path,
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

const REQUEST_LIMIT: usize = 8 * 1024;
const RESPONSE_LIMIT: usize = 32 * 1024;
const DEADLINE: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// The operation requested by the editor.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Correct language and supported formatting errors.
    Correct,
    /// Group related ideas within the selected passage.
    Organize,
}

#[derive(Serialize)]
struct Request<'a> {
    action: Action,
    markdown: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    units: Option<&'a [Range<usize>]>,
}

/// A model notice whose source quote the editor must check before display.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notice {
    /// The exact words from the original selection.
    pub source: String,
    /// A short description of the unresolved point.
    pub message: String,
}

/// A decoded response from a helper that exited successfully.
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    /// A proposed revision for the editor to validate.
    Ok {
        /// The proposed replacement for the selected Markdown.
        markdown: String,
        /// Any unresolved points reported by the model.
        notices: Vec<Notice>,
    },
    /// The platform is ineligible for Apple Intelligence.
    DeviceIneligible {},
    /// Apple Intelligence is disabled in System Settings.
    IntelligenceDisabled {},
    /// The installed system model is temporarily unready.
    ModelNotReady {},
    /// The system model is unavailable for another reason.
    ModelUnavailable {},
    /// The selection exceeds the available model context.
    InputTooLong {},
    /// The model declined the request.
    Refused {},
    /// The model did not produce a complete revision.
    InvalidResult {},
    /// Generation failed for another reason.
    GenerationFailed {},
}

/// A failure in the client, process, or transport.
#[derive(Debug)]
pub enum Error {
    /// The executable must be resolved from the application bundle.
    RelativeHelperPath,
    /// The request could not be encoded.
    InvalidRequest,
    /// The encoded request exceeds the transport limit.
    RequestTooLarge,
    /// The helper could not be spawned.
    Start(io::Error),
    /// Process monitoring or pipe I/O failed.
    Transport(io::Error),
    /// The helper terminated unsuccessfully.
    Exit(ExitStatus),
    /// The response exceeds the transport limit.
    ResponseTooLarge,
    /// The response is missing or violates the wire format.
    InvalidResponse,
    /// The editor cancelled the request.
    Cancelled,
    /// The helper did not complete within the deadline.
    Timeout,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativeHelperPath => f.write_str("revision helper path must be absolute"),
            Self::InvalidRequest => f.write_str("could not encode revision request"),
            Self::RequestTooLarge => f.write_str("revision request exceeds byte limit"),
            Self::Start(_) => f.write_str("could not start revision helper"),
            Self::Transport(_) => f.write_str("revision helper transport failed"),
            Self::Exit(status) => write!(f, "revision helper exited unsuccessfully ({status})"),
            Self::ResponseTooLarge => f.write_str("revision response exceeds byte limit"),
            Self::InvalidResponse => f.write_str("invalid revision helper response"),
            Self::Cancelled => f.write_str("revision cancelled"),
            Self::Timeout => f.write_str("revision helper timed out"),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Start(error) | Self::Transport(error) => Some(error),
            _ => None,
        }
    }
}

/// Sends selected Markdown to the helper and waits for a bounded response.
///
/// Call from a background worker. Setting `cancelled` stops the request and
/// waits for the child and its pipe workers to exit before returning.
/// Organization can include parsed UTF-8 ranges in `units`.
///
/// # Errors
/// Returns a transport error if the helper cannot complete a valid exchange.
/// Model outcomes, including unavailability, are returned as responses.
pub fn revise(
    helper: &Path,
    action: Action,
    markdown: &str,
    units: Option<&[Range<usize>]>,
    cancelled: &AtomicBool,
) -> Result<Response, Error> {
    if !helper.is_absolute() {
        return Err(Error::RelativeHelperPath);
    }
    if markdown.len() > REQUEST_LIMIT {
        return Err(Error::RequestTooLarge);
    }
    let request = serde_json::to_vec(&Request {
        action,
        markdown,
        units,
    })
    .map_err(|_| Error::InvalidRequest)?;
    if request.len() > REQUEST_LIMIT {
        return Err(Error::RequestTooLarge);
    }
    exchange(Command::new(helper), &request, cancelled, DEADLINE)
}

enum PipeEvent {
    Written,
    Read(Vec<u8>),
    Failed(Error),
}

fn exchange(
    mut command: Command,
    request: &[u8],
    cancelled: &AtomicBool,
    timeout: Duration,
) -> Result<Response, Error> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(Error::Start)?;
    let deadline = Instant::now() + timeout;

    let bytes = thread::scope(|scope| {
        let transfer = (|| {
            let mut stdin = child.stdin.take().ok_or_else(missing_pipe)?;
            let mut stdout = child.stdout.take().ok_or_else(missing_pipe)?;
            let (events, receiver) = mpsc::channel();
            let writes = events.clone();
            thread::Builder::new()
                .spawn_scoped(scope, move || {
                    let result = stdin.write_all(request);
                    drop(stdin);
                    let _ = writes.send(match result {
                        Ok(()) => PipeEvent::Written,
                        Err(error) => PipeEvent::Failed(Error::Transport(error)),
                    });
                })
                .map_err(Error::Transport)?;
            thread::Builder::new()
                .spawn_scoped(scope, move || {
                    let mut output = Vec::new();
                    let result = stdout
                        .by_ref()
                        .take((RESPONSE_LIMIT + 1) as u64)
                        .read_to_end(&mut output);
                    let _ = events.send(match result {
                        Ok(_) if output.len() <= RESPONSE_LIMIT => PipeEvent::Read(output),
                        Ok(_) => PipeEvent::Failed(Error::ResponseTooLarge),
                        Err(error) => PipeEvent::Failed(Error::Transport(error)),
                    });
                })
                .map_err(Error::Transport)?;

            let mut written = false;
            let mut output = None;
            loop {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                if Instant::now() >= deadline {
                    return Err(Error::Timeout);
                }
                let exit = child.try_wait().map_err(Error::Transport)?;
                for event in receiver.try_iter() {
                    match event {
                        PipeEvent::Written => written = true,
                        PipeEvent::Read(bytes) => output = Some(bytes),
                        PipeEvent::Failed(error) => return Err(error),
                    }
                }
                if let Some(status) = exit
                    && !status.success()
                {
                    return Err(Error::Exit(status));
                }
                if exit.is_some()
                    && written
                    && let Some(bytes) = output.take()
                {
                    return Ok(bytes);
                }
                thread::sleep(POLL_INTERVAL);
            }
        })();
        if transfer.is_err() {
            // Closing the child's ends releases any blocked pipe workers before the scope joins.
            let _ = child.kill();
            child.wait().map_err(Error::Transport)?;
        }
        transfer
    })?;

    if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
        return Err(Error::InvalidResponse);
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::InvalidResponse)
}

fn missing_pipe() -> Error {
    Error::Transport(io::Error::other("missing child pipe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUEST: &[u8] = br#"{"action":"correct","markdown":"Riunone"}"#;
    const SUCCESS: &str = r#"{"status":"ok","markdown":"Riunione","notices":[]}"#;

    fn script(source: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", source]);
        command
    }

    #[test]
    fn transfers_json_and_distinguishes_model_outcomes() {
        let success = format!(
            "IFS= read -r request; [ \"$request\" = '{}' ] || exit 7; printf '%s' '{SUCCESS}'",
            str::from_utf8(REQUEST).unwrap()
        );
        let response = exchange(script(&success), REQUEST, &AtomicBool::new(false), DEADLINE);
        assert!(matches!(response, Ok(Response::Ok { markdown, notices })
            if markdown == "Riunione" && notices.is_empty()));

        let outcomes = [
            "device_ineligible",
            "intelligence_disabled",
            "model_not_ready",
            "model_unavailable",
            "input_too_long",
            "refused",
            "invalid_result",
            "generation_failed",
        ];
        for status in outcomes {
            let fake = format!("IFS= read -r request; printf '%s' '{{\"status\":\"{status}\"}}'");
            assert!(
                exchange(script(&fake), REQUEST, &AtomicBool::new(false), DEADLINE).is_ok(),
                "{status}"
            );
        }
    }

    #[test]
    fn rejects_invalid_oversized_and_unsuccessful_responses() {
        let cases = [
            "{",
            "[]",
            r#"["model_not_ready"]"#,
            r#"{"status":"ok","markdown":"Riunione"}"#,
            r#"{"status":"refused","markdown":"unexpected"}"#,
            r#"{"status":"ok","markdown":"Riunione","notices":[]}{}"#,
        ];
        for output in cases {
            let fake = format!("IFS= read -r request; printf '%s' '{output}'");
            let result = exchange(script(&fake), REQUEST, &AtomicBool::new(false), DEADLINE);
            assert!(matches!(result, Err(Error::InvalidResponse)), "{output}");
        }

        let fake = format!("IFS= read -r request; printf '%s' '{SUCCESS}'; exit 9");
        assert!(
            matches!(exchange(script(&fake), REQUEST, &AtomicBool::new(false), DEADLINE), Err(Error::Exit(status)) if status.code() == Some(9))
        );
        assert!(
            matches!(exchange(script("IFS= read -r request; kill -s KILL $$"), REQUEST, &AtomicBool::new(false), DEADLINE), Err(Error::Exit(status)) if status.code().is_none())
        );
        assert!(matches!(
            exchange(
                script("IFS= read -r request; exec /bin/cat /dev/zero"),
                REQUEST,
                &AtomicBool::new(false),
                DEADLINE
            ),
            Err(Error::ResponseTooLarge)
        ));
        assert!(matches!(
            revise(
                Path::new("/dev/null/polar-helper"),
                Action::Correct,
                "Text",
                None,
                &AtomicBool::new(false)
            ),
            Err(Error::Start(_))
        ));
        assert!(matches!(
            revise(
                Path::new("/bin/sh"),
                Action::Correct,
                &"\n".repeat(REQUEST_LIMIT),
                None,
                &AtomicBool::new(false)
            ),
            Err(Error::RequestTooLarge)
        ));
    }

    #[test]
    fn times_out_or_cancels_while_pipes_are_blocked() {
        let timeout = Duration::from_millis(150);
        let large_input = vec![b'a'; 1024 * 1024];
        for (fake, input) in [
            ("exec /bin/sleep 30", large_input.as_slice()),
            ("IFS= read -r request; exec /bin/sleep 30", REQUEST),
        ] {
            let started = Instant::now();
            let result = exchange(script(fake), input, &AtomicBool::new(false), timeout);
            assert!(matches!(result, Err(Error::Timeout)), "{fake}");
            assert!(started.elapsed() < Duration::from_secs(5), "{fake}");
        }

        let cancelled = AtomicBool::new(false);
        thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(timeout);
                cancelled.store(true, Ordering::Relaxed);
            });
            let result = exchange(
                script("exec /bin/sleep 30"),
                &large_input,
                &cancelled,
                Duration::from_secs(5),
            );
            assert!(matches!(result, Err(Error::Cancelled)));
        });
        let retry = format!("IFS= read -r request; printf '%s' '{SUCCESS}'");
        assert!(exchange(script(&retry), REQUEST, &AtomicBool::new(false), DEADLINE).is_ok());
    }
}
