use a2amx::harness::Harness;

fn infer(command: &[&str]) -> Harness {
    let command: Vec<String> = command.iter().map(|word| (*word).to_owned()).collect();
    Harness::infer(&command)
}

#[test]
fn names_a_harness_by_executable_file_name() {
    assert_eq!(infer(&["claude"]), Harness::Claude);
    assert_eq!(infer(&["/usr/bin/claude", "--resume"]), Harness::Claude);
    assert_eq!(infer(&["./codex"]), Harness::Codex);
    assert_eq!(infer(&["omp"]), Harness::Omp);
}

#[test]
fn anything_else_is_generic() {
    assert_eq!(infer(&["sh"]), Harness::Generic);
    assert_eq!(infer(&["Claude"]), Harness::Generic);
    assert_eq!(infer(&["claude-code"]), Harness::Generic);
    assert_eq!(infer(&["env", "FOO=1", "claude"]), Harness::Generic);
    assert_eq!(infer(&["/"]), Harness::Generic);
    assert_eq!(infer(&[]), Harness::Generic);
}

#[test]
fn needs_input_requires_both_claude_dialog_markers_on_one_row() {
    use a2amx::emulator::{Emulator, Size};
    use a2amx::harness::needs_input;

    let mut emulator = Emulator::new(Size { cols: 40, rows: 5 });
    emulator.feed(" Enter to confirm · Esc to cancel".as_bytes());
    let dialog = emulator.screen();
    assert!(needs_input(Harness::Claude, &dialog));
    for harness in [Harness::Omp, Harness::Codex, Harness::Generic] {
        assert!(!needs_input(harness, &dialog));
    }
    for output in [
        "Enter to confirm\r\nEsc to cancel",
        "Enter to confirm",
        "Esc to cancel",
        "\x1b[?2004h\x1b[2;1H────────────────────────────────────────\x1b[3;1H❯ \x1b[4;1H────────────────────────────────────────\x1b[3;3H",
        "",
    ] {
        let mut emulator = Emulator::new(Size { cols: 40, rows: 5 });
        emulator.feed(output.as_bytes());
        assert!(
            !needs_input(Harness::Claude, &emulator.screen()),
            "{output:?}"
        );
    }
}

#[test]
fn codex_needs_input_maps_only_missing_thread_and_approval() {
    use a2amx::harness::label_for_codex_reason;

    for (reason, expected) in [
        (Some("no_thread"), Some("needs_input")),
        (Some("waiting_on_approval"), Some("needs_input")),
        (Some("in_flight"), None),
        (Some("app_server_down"), None),
        (Some("thread_error"), None),
        (None, None),
    ] {
        assert_eq!(label_for_codex_reason(reason), expected, "{reason:?}");
    }
}
