//! An interactive shell over a WebSocket (PLAN §6, M6). A client types into a session and
//! reads what the shell prints, as a terminal would. The route is
//! `GET /v1/sessions/{id}/pty`. The protocol is JSON text frames:
//!
//! - client → server: `{"type":"input","data":"..."}` for typed text, and
//!   `{"type":"resize","cols":N,"rows":N}` for the terminal size.
//! - server → client: `{"type":"output","data":"..."}` for what the shell prints, and
//!   `{"type":"exit","code":N}` when the shell ends, followed by a close frame.
//!
//! Typed text is edited on the server, as a line discipline does: characters are echoed,
//! Backspace erases, Enter runs the line, Ctrl-C discards the line or stops a running command,
//! and Ctrl-D on an empty line ends the shell. `exit [N]` ends it with status N. A running
//! command is stopped by dropping it at its next await point, as a cancelled exec is (ADR 0032).
//! Input typed while a command runs is discarded, except Ctrl-C.

use axum::extract::ws::{Message, WebSocket};
use futures::future::BoxFuture;
use futures::{SinkExt, StreamExt};
use sandbox_types::{ExecResult, SandboxError};
use serde::Deserialize;
use std::sync::Arc;

/// What the shell prints before each line.
pub const PROMPT: &str = "$ ";
const CTRL_C: char = '\u{3}';
const CTRL_D: char = '\u{4}';
const BACKSPACE: char = '\u{8}';
const DELETE: char = '\u{7f}';

/// What typed text produced, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typed {
    /// Text to show the user.
    Echo(String),
    /// A finished line, without its newline.
    Line(String),
    /// Ctrl-C: the line was discarded.
    Interrupt,
    /// Ctrl-D on an empty line.
    EndOfInput,
}

/// The line discipline: edits the line being typed and reports what to echo and what to run.
#[derive(Debug, Default)]
pub struct LineEditor {
    line: String,
}

impl LineEditor {
    /// Applies typed text and returns the events it produced, in order.
    pub fn feed(&mut self, input: &str) -> Vec<Typed> {
        let mut events = Vec::new();
        for c in input.chars() {
            match c {
                '\r' | '\n' => {
                    let line = std::mem::take(&mut self.line);
                    events.push(Typed::Echo("\r\n".to_string()));
                    events.push(Typed::Line(line));
                }
                CTRL_C => {
                    self.line.clear();
                    events.push(Typed::Echo("^C\r\n".to_string()));
                    events.push(Typed::Interrupt);
                }
                CTRL_D => {
                    if self.line.is_empty() {
                        events.push(Typed::EndOfInput);
                    }
                }
                BACKSPACE | DELETE => {
                    if self.line.pop().is_some() {
                        events.push(Typed::Echo("\u{8} \u{8}".to_string()));
                    }
                }
                c if c.is_control() => {}
                c => {
                    self.line.push(c);
                    events.push(Typed::Echo(c.to_string()));
                }
            }
        }
        events
    }
}

/// The status of a line that is `exit` or `exit N`, or `None` for any other line. `exit`
/// with no number ends the shell with status 0, since the shell does not carry the last status
/// here.
pub fn exit_status(line: &str) -> Option<i32> {
    let rest = line.trim().strip_prefix("exit")?;
    if rest.is_empty() {
        return Some(0);
    }
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    rest.trim().parse().ok()
}

/// A message from the client.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

/// What the server does for the shell: runs each line, and does what an exec does after it. The
/// driver holds no session state of its own, so the server keeps the trace and the checkpoints.
pub trait Runner: Send + Sync {
    /// Runs one line. Dropping the returned future stops the command at its next await point.
    fn run<'a>(&'a self, line: &'a str) -> BoxFuture<'a, Result<ExecResult, SandboxError>>;
    /// Runs after a line, whether it finished or was stopped. `outcome` is `None` when the line was
    /// stopped, so the partial writes it made are still checkpointed.
    fn finish<'a>(
        &'a self,
        line: &'a str,
        outcome: Option<&'a Result<ExecResult, SandboxError>>,
    ) -> BoxFuture<'a, Result<(), SandboxError>>;
    /// Sets the terminal size the session reports through `COLUMNS` and `LINES`.
    fn resize<'a>(&'a self, cols: u16, rows: u16) -> BoxFuture<'a, ()>;
}

fn output(data: &str) -> Message {
    Message::Text(
        serde_json::json!({ "type": "output", "data": data })
            .to_string()
            .into(),
    )
}

fn exit(code: i32) -> Message {
    Message::Text(
        serde_json::json!({ "type": "exit", "code": code })
            .to_string()
            .into(),
    )
}

/// Whether a client message is Ctrl-C typed while a command runs.
fn is_interrupt(text: &str) -> bool {
    matches!(
        serde_json::from_str::<ClientMessage>(text),
        Ok(ClientMessage::Input { data }) if data.contains(CTRL_C)
    )
}

/// Runs one line. While it runs, only Ctrl-C from the client has an effect. Returns `None` when
/// Ctrl-C, or the end of the connection, stopped the line.
async fn run_line<S>(
    runner: &dyn Runner,
    line: &str,
    rx: &mut S,
) -> Option<Result<ExecResult, SandboxError>>
where
    S: futures::Stream<Item = Result<Message, axum::Error>> + Unpin,
{
    let run = runner.run(line);
    tokio::pin!(run);
    loop {
        tokio::select! {
            result = &mut run => return Some(result),
            message = rx.next() => match message {
                Some(Ok(Message::Text(text))) if is_interrupt(&text) => return None,
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
                // Other input while a command runs is discarded.
                Some(Ok(_)) => {}
            },
        }
    }
}

/// Serves an interactive shell on `socket` until the shell exits or the client leaves.
pub async fn serve(socket: WebSocket, runner: Arc<dyn Runner>) {
    let (mut tx, mut rx) = socket.split();
    let mut editor = LineEditor::default();
    let mut size: Option<(u16, u16)> = None;
    if tx.send(output(PROMPT)).await.is_err() {
        return;
    }
    while let Some(Ok(message)) = rx.next().await {
        let text = match message {
            Message::Text(text) => text,
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(message) = serde_json::from_str::<ClientMessage>(&text) else {
            continue;
        };
        match message {
            ClientMessage::Resize { cols, rows } => {
                size = Some((cols, rows));
                runner.resize(cols, rows).await;
            }
            ClientMessage::Input { data } => {
                for typed in editor.feed(&data) {
                    match typed {
                        Typed::Echo(text) => {
                            if tx.send(output(&text)).await.is_err() {
                                return;
                            }
                        }
                        Typed::Interrupt => {
                            if tx.send(output(PROMPT)).await.is_err() {
                                return;
                            }
                        }
                        Typed::EndOfInput => {
                            let _ = tx.send(exit(0)).await;
                            return;
                        }
                        Typed::Line(line) => {
                            if let Some(code) = exit_status(&line) {
                                let _ = tx.send(exit(code)).await;
                                return;
                            }
                            if let Some((cols, rows)) = size {
                                runner.resize(cols, rows).await;
                            }
                            let ran = run_line(runner.as_ref(), &line, &mut rx).await;
                            if let Err(error) = runner.finish(&line, ran.as_ref()).await {
                                let _ = tx.send(output(&format!("{}\r\n", error.message))).await;
                                let _ = tx.send(exit(1)).await;
                                return;
                            }
                            let sent = match ran {
                                Some(Ok(out)) => {
                                    send_bytes(&mut tx, &out.stdout).await
                                        && send_bytes(&mut tx, &out.stderr).await
                                }
                                Some(Err(error)) => tx
                                    .send(output(&format!("{}\r\n", error.message)))
                                    .await
                                    .is_ok(),
                                // Ctrl-C was read while the command ran, so echo it here.
                                None => tx.send(output("^C\r\n")).await.is_ok(),
                            };
                            if !sent || tx.send(output(PROMPT)).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        }
    }
    let _ = tx.send(Message::Close(None)).await;
}

/// Sends shell output bytes as text, replacing any invalid UTF-8. Returns false when the
/// client has gone.
async fn send_bytes<T>(tx: &mut T, bytes: &[u8]) -> bool
where
    T: futures::Sink<Message> + Unpin,
{
    if bytes.is_empty() {
        return true;
    }
    // A terminal needs CR LF line ends, and the shell writes bare LF.
    let text = String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .replace('\n', "\r\n");
    tx.send(output(&text)).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echoed(events: &[Typed]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                Typed::Echo(s) => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn typed_characters_are_echoed_and_form_the_line() {
        let mut editor = LineEditor::default();
        let events = editor.feed("echo hi");
        assert_eq!(echoed(&events), "echo hi");
        assert!(events.iter().all(|e| matches!(e, Typed::Echo(_))));
        let events = editor.feed("\r");
        assert_eq!(
            events,
            vec![Typed::Echo("\r\n".into()), Typed::Line("echo hi".into())]
        );
    }

    #[test]
    fn backspace_erases_the_last_character() {
        let mut editor = LineEditor::default();
        editor.feed("ecx");
        let events = editor.feed("\u{7f}ho\n");
        assert_eq!(events.last(), Some(&Typed::Line("echo".into())));
    }

    #[test]
    fn backspace_on_an_empty_line_echoes_nothing() {
        let mut editor = LineEditor::default();
        assert!(editor.feed("\u{8}").is_empty());
    }

    #[test]
    fn ctrl_c_discards_the_line_and_reports_an_interrupt() {
        let mut editor = LineEditor::default();
        editor.feed("sleep 9");
        let events = editor.feed("\u{3}");
        assert_eq!(events, vec![Typed::Echo("^C\r\n".into()), Typed::Interrupt]);
        // The discarded text is not run.
        assert_eq!(editor.feed("\n").last(), Some(&Typed::Line(String::new())));
    }

    #[test]
    fn ctrl_d_ends_input_only_on_an_empty_line() {
        let mut editor = LineEditor::default();
        editor.feed("x");
        assert!(
            editor.feed("\u{4}").is_empty(),
            "Ctrl-D with text does nothing"
        );
        editor.feed("\n");
        assert_eq!(editor.feed("\u{4}"), vec![Typed::EndOfInput]);
    }

    #[test]
    fn several_lines_in_one_message_come_out_in_order() {
        let mut editor = LineEditor::default();
        let lines: Vec<String> = editor
            .feed("one\rtwo\r")
            .into_iter()
            .filter_map(|e| match e {
                Typed::Line(l) => Some(l),
                _ => None,
            })
            .collect();
        assert_eq!(lines, ["one", "two"]);
    }

    #[test]
    fn control_characters_other_than_the_ones_handled_are_ignored() {
        let mut editor = LineEditor::default();
        assert!(editor.feed("\u{1b}").is_empty());
    }

    #[test]
    fn exit_takes_an_optional_status() {
        assert_eq!(exit_status("exit"), Some(0));
        assert_eq!(exit_status("exit 3"), Some(3));
        assert_eq!(exit_status("  exit 7  "), Some(7));
        assert_eq!(exit_status("exit x"), None);
        assert_eq!(exit_status("exitcode"), None);
        assert_eq!(exit_status("echo exit"), None);
    }

    #[test]
    fn an_interrupt_is_recognised_only_as_typed_input() {
        let typed = serde_json::json!({"type": "input", "data": "\u{3}"}).to_string();
        assert!(is_interrupt(&typed));
        let output_text = serde_json::json!({"type": "resize", "cols": 1, "rows": 1}).to_string();
        assert!(!is_interrupt(&output_text));
        assert!(!is_interrupt("not json"));
    }
}
