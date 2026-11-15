//! Unit tests moved out of src/pty.rs (renamed to avoid the existing tests/pty.rs integration test).

use sandbox_server::pty::{exit_status, is_interrupt, LineEditor, Typed};

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
    assert_eq!(exit_status("exit", 0), Some(0));
    assert_eq!(
        exit_status("exit", 5),
        Some(5),
        "a bare exit reports the last status"
    );
    assert_eq!(exit_status("exit 3", 5), Some(3));
    assert_eq!(exit_status("  exit 7  ", 0), Some(7));
    assert_eq!(exit_status("exit x", 0), None);
    assert_eq!(exit_status("exitcode", 0), None);
    assert_eq!(exit_status("echo exit", 0), None);
}

#[test]
fn an_interrupt_is_recognised_only_as_typed_input() {
    let typed = serde_json::json!({"type": "input", "data": "\u{3}"}).to_string();
    assert!(is_interrupt(&typed));
    let output_text = serde_json::json!({"type": "resize", "cols": 1, "rows": 1}).to_string();
    assert!(!is_interrupt(&output_text));
    assert!(!is_interrupt("not json"));
}
