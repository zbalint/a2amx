use super::*;

fn screen_with(rows: u16, lines: &[(u16, &str)]) -> a2amx::emulator::Screen {
    let mut emulator = Emulator::new(Size { cols: 169, rows });
    for (row, text) in lines {
        emulator.feed(format!("\x1b[{row};1H{text}").as_bytes());
    }
    emulator.screen()
}

fn read_with(harness: Harness, rows: u16, lines: &[(u16, &str)]) -> Option<QuotaInfo> {
    quota::read(&screen_with(rows, lines), harness)
}

fn read(rows: u16, lines: &[(u16, &str)]) -> Option<QuotaInfo> {
    read_with(Harness::Codex, rows, lines)
}

fn info(five_hour_left: Option<u8>, weekly_left: Option<u8>) -> Option<QuotaInfo> {
    Some(QuotaInfo {
        five_hour_left,
        weekly_left,
        ..Default::default()
    })
}

#[test]
fn reads_omp_usage_limit_error() {
    let screen = screen_with(
        51,
        &[(
            41,
            "  Error: Codex error event: The usage limit has been reached (code=usage_limit_reached)",
        )],
    );
    assert_eq!(
        quota::read(&screen, Harness::Omp).and_then(|quota| quota.exhausted()),
        Some("usage")
    );
}

#[test]
fn reads_omp_wrapped_usage_limit_error() {
    let lines = [
        (
            43,
            "  Error: Retry failed after 1 attempts: ... Original error: Codex error event: The usage limit has",
        ),
        (44, "  been reached (code=usage_limit_reached)"),
    ];
    assert_eq!(
        read_with(Harness::Omp, 51, &lines),
        Some(QuotaInfo {
            limit_reached: true,
            ..Default::default()
        })
    );
}

#[test]
fn omp_error_outside_last_twelve_rows_is_ignored() {
    let text =
        "  Error: Codex error event: The usage limit has been reached (code=usage_limit_reached)";
    assert_eq!(read_with(Harness::Omp, 51, &[(19, text)]), None);
}

#[test]
fn omp_advisor_warning_is_not_a_limit() {
    let text = "  Warning: advisor: Advisor \"Advisor\" quota exhausted — pausing until reset.";
    assert_eq!(read_with(Harness::Omp, 51, &[(41, text)]), None);
}

#[test]
fn omp_other_error_is_not_a_limit() {
    let text = "  Error: Codex error event: overloaded (code=server_error)";
    assert_eq!(read_with(Harness::Omp, 51, &[(41, text)]), None);
}

#[test]
fn omp_quoted_error_is_not_a_limit() {
    let text = "  the log said Error: Codex error event: usage (code=usage_limit_reached)";
    assert_eq!(read_with(Harness::Omp, 51, &[(41, text)]), None);
}

#[test]
fn omp_nonadjacent_code_is_not_a_limit() {
    assert_eq!(
        read_with(
            Harness::Omp,
            51,
            &[(41, "Error: x"), (43, "(code=usage_limit_reached)")],
        ),
        None
    );
}

#[test]
fn generic_does_not_read_omp_error() {
    let text =
        "  Error: Codex error event: The usage limit has been reached (code=usage_limit_reached)";
    assert_eq!(read_with(Harness::Generic, 51, &[(41, text)]), None);
}

#[test]
fn empty_omp_screen_has_no_limit() {
    assert_eq!(read_with(Harness::Omp, 51, &[]), None);
}

#[test]
fn reads_both_windows_from_a_codex_status_line() {
    let text = "gpt-5 · repo · Ready · 40% context · weekly 12% left · 1.2M used · 5h 0% left";
    assert_eq!(read(24, &[(24, text)]), info(Some(0), Some(12)));
}

#[test]
fn reads_both_windows_from_a_claude_status_line() {
    let text = "[Sonnet] repo | main | $1.20 | 45k/200k (22%) | 5h 7% left 7d 63% left";
    assert_eq!(read(24, &[(24, text)]), info(Some(7), Some(63)));
}

#[test]
fn accepts_one_hundred_and_rejects_more() {
    assert_eq!(read(24, &[(24, "5h 100% left")]), info(Some(100), None));
    assert_eq!(read(24, &[(24, "5h 101% left")]), None);
}

#[test]
fn rejects_malformed_tokens() {
    for text in [
        "5h left",
        "5h 3%left",
        "5h 3% leftover",
        "x5h 3% left",
        "5h  3% left",
    ] {
        assert_eq!(read(24, &[(24, text)]), None, "{text}");
    }
}

#[test]
fn looks_only_at_the_last_three_rows() {
    assert_eq!(read(24, &[(19, "5h 0% left")]), None);
    assert_eq!(read(24, &[(22, "5h 0% left")]), info(Some(0), None));
}

#[test]
fn the_bottom_row_wins() {
    assert_eq!(
        read(24, &[(24, "5h 40% left"), (23, "5h 0% left")]),
        info(Some(40), None)
    );
}

#[test]
fn short_and_empty_screens_are_handled() {
    assert_eq!(read(2, &[(1, "5h 9% left")]), info(Some(9), None));
    assert_eq!(read(24, &[]), None);
}
