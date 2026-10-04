use a2amx::harness::Harness;
use a2amx::status;
use a2amx::wire::{Activity, QuotaInfo, StatusInfo};

const PRE: &str = "\x1b7\x1b[23;1H\x1b[0m\x1b[2K\x1b[38;5;240m";
const BAR: &str = "\x1b[0m\x1b[24;1H\x1b[0m\x1b[38;5;252;48;5;236m";
const POST: &str = "\x1b[0m\x1b8";

fn info(address: &str) -> StatusInfo {
    StatusInfo {
        address: address.into(),
        pending: 0,
        hold: None,
        harness: None,
        activity: None,
        quota: None,
    }
}

fn visible(bytes: &[u8]) -> String {
    let bar = b"\x1b[24;1H";
    let Some(start) = bytes.windows(bar.len()).position(|window| window == bar) else {
        panic!("bar must draw on row 24");
    };
    let bytes = &bytes[start..];
    let mut bytes = bytes.iter().copied();
    let mut text = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte != 0x1b {
            text.push(byte);
        } else if bytes.next() == Some(b'[') {
            for byte in bytes.by_ref() {
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
        }
    }
    String::from_utf8_lossy(&text).into_owned()
}

#[test]
fn details_drop_in_priority_order_as_the_terminal_narrows() {
    let mut info = info("agent-plan@host-a");
    info.activity = Some(Activity::Working);
    info.harness = Some(Harness::Claude);
    info.pending = 2;
    info.quota = Some(QuotaInfo {
        five_hour: Some(92),
        weekly: Some(42),
        limit_reached: false,
    });
    // E1: ordinary quota is hidden.
    assert_eq!(
        visible(&status::render("s1", Some(&info), 2, 80, 24)),
        format!(
            " agent-plan@host-a │ ● working │ claude {} 2 pending ",
            " ".repeat(28)
        )
    );
    info.quota = Some(QuotaInfo {
        five_hour: Some(92),
        weekly: Some(12),
        limit_reached: false,
    });
    // E2-E6: harness, quota, activity, then pending are removed.
    for (cols, left, padding, right) in [
        (
            80,
            " agent-plan@host-a │ ● working │ 5h 92% wk 12% │ claude ",
            12,
            " 2 pending ",
        ),
        (
            60,
            " agent-plan@host-a │ ● working │ 5h 92% wk 12% ",
            1,
            " 2 pending ",
        ),
        (50, " agent-plan@host-a │ ● working ", 7, " 2 pending "),
        (36, " agent-plan@host-a ", 5, " 2 pending "),
        (28, " agent-plan@host-a ", 8, ""),
    ] {
        assert_eq!(
            visible(&status::render("s1", Some(&info), 2, cols, 24)),
            format!("{left}{}{right}", " ".repeat(padding)),
            "cols {cols}"
        );
    }
}

#[test]
fn held_alert_keeps_priority_and_only_hints_releasable_holds() {
    let mut info = info("agent-plan@host-a");
    info.activity = Some(Activity::Working);
    info.harness = Some(Harness::Claude);
    info.pending = 2;
    info.quota = Some(QuotaInfo {
        five_hour: Some(92),
        weekly: Some(42),
        limit_reached: false,
    });
    info.hold = Some("human_draft".into());
    // E7-E9: activity is hidden while held; pending precedes the hint in removal order.
    for (cols, left, padding, right) in [
        (
            100,
            " agent-plan@host-a │ claude ",
            35,
            " 2 pending  HELD human_draft · ^B r ",
        ),
        (50, " agent-plan@host-a ", 5, " HELD human_draft · ^B r "),
        (40, " agent-plan@host-a ", 2, " HELD human_draft "),
    ] {
        assert_eq!(
            visible(&status::render("s1", Some(&info), 2, cols, 24)),
            format!("{left}{}{right}", " ".repeat(padding))
        );
    }
    info.pending = 0;
    info.harness = None;
    info.activity = None;
    info.quota = None;
    // E10-E11 and B5: clipped name spans must not emit empty bold sequences.
    assert_eq!(
        visible(&status::render("s1", Some(&info), 2, 20, 24)),
        "  HELD human_draft "
    );
    assert_eq!(
        status::render("s1", Some(&info), 2, 20, 24),
        format!(
            "{PRE}{}{BAR} \x1b[0m\x1b[1;37;41m HELD human_draft {POST}",
            "─".repeat(19)
        )
        .as_bytes()
    );
    assert_eq!(
        visible(&status::render("s1", Some(&info), 2, 8, 24)),
        " HELD h"
    );
    assert_eq!(
        status::render("s1", Some(&info), 2, 8, 24),
        format!(
            "{PRE}{}{BAR}\x1b[0m\x1b[1;37;41m HELD h{POST}",
            "─".repeat(7)
        )
        .as_bytes()
    );
    // E12: a future hold not released by r gets no hint.
    info.hold = Some("channel_down".into());
    assert_eq!(
        visible(&status::render("s1", Some(&info), 2, 60, 24)),
        format!(" agent-plan@host-a {} HELD channel_down ", " ".repeat(21))
    );
}

#[test]
fn low_quota_boundary_resetting_and_held_activity_match_the_bar_contract() {
    let mut info = info("a");
    info.activity = Some(Activity::Idle);
    info.quota = Some(QuotaInfo {
        limit_reached: true,
        ..Default::default()
    });
    // E13.
    assert_eq!(
        visible(&status::render("s1", Some(&info), 2, 40, 24)),
        format!(" a │ ● idle │ limit {}", " ".repeat(19))
    );
    info.activity = None;
    info.quota = Some(QuotaInfo {
        five_hour: Some(15),
        ..Default::default()
    });
    // E14-E15.
    assert_eq!(
        visible(&status::render("s1", Some(&info), 2, 40, 24)),
        format!(" a │ 5h 15% {}", " ".repeat(27))
    );
    info.quota = Some(QuotaInfo {
        five_hour: Some(16),
        ..Default::default()
    });
    assert_eq!(
        visible(&status::render("s1", Some(&info), 2, 40, 24)),
        format!(" a {}", " ".repeat(36))
    );
    info.quota = None;
    info.activity = Some(Activity::Busy);
    // E16.
    assert_eq!(
        visible(&status::render("s1", Some(&info), 2, 40, 24)),
        format!(" a │ ● resetting {}", " ".repeat(22))
    );
    info.activity = Some(Activity::Working);
    info.hold = Some("human_draft".into());
    // E17.
    assert_eq!(
        visible(&status::render("s1", Some(&info), 2, 60, 24)),
        format!(" a {} HELD human_draft · ^B r ", " ".repeat(31))
    );
}

#[test]
fn styled_bar_bytes_restore_cursor_and_foreground() {
    // B1-B3.
    assert_eq!(
        status::render("s1", None, 2, 40, 24),
        format!(
            "{PRE}{}{BAR} \x1b[1ms1\x1b[22m {}{POST}",
            "─".repeat(39),
            " ".repeat(35)
        )
        .as_bytes()
    );
    let mut info = info("a");
    info.activity = Some(Activity::Working);
    assert_eq!(status::render("s1", Some(&info), 2, 40, 24), format!("{PRE}{}{BAR} \x1b[1ma\x1b[22m \x1b[38;5;240m│\x1b[38;5;252m \x1b[38;5;221m●\x1b[38;5;252m working {}{POST}", "─".repeat(39), " ".repeat(24)).as_bytes());
    info.activity = None;
    info.hold = Some("channel_down".into());
    assert_eq!(
        status::render("s1", Some(&info), 2, 40, 24),
        format!(
            "{PRE}{}{BAR} \x1b[1ma\x1b[22m {}\x1b[0m\x1b[1;37;41m HELD channel_down {POST}",
            "─".repeat(39),
            " ".repeat(17)
        )
        .as_bytes()
    );
    // B4.
    info.hold = None;
    for (activity, color) in [(Activity::Idle, 78), (Activity::Busy, 209)] {
        info.activity = Some(activity);
        let rendered = status::render("s1", Some(&info), 2, 40, 24);
        let bytes = String::from_utf8_lossy(&rendered);
        assert!(bytes.contains(&format!("\x1b[38;5;{color}m●\x1b[38;5;252m")));
    }
    info.activity = None;
    for (quota, span) in [
        (
            QuotaInfo {
                five_hour: Some(15),
                ..Default::default()
            },
            "\x1b[38;5;221m5h 15%\x1b[38;5;252m",
        ),
        (
            QuotaInfo {
                limit_reached: true,
                ..Default::default()
            },
            "\x1b[38;5;203mlimit\x1b[38;5;252m",
        ),
    ] {
        info.quota = Some(quota);
        let rendered = status::render("s1", Some(&info), 2, 40, 24);
        let bytes = String::from_utf8_lossy(&rendered);
        assert!(bytes.contains(span));
    }
}

#[test]
fn release_hint_respects_prefix_range_and_each_releasable_reason() {
    let mut info = info("a");
    for reason in [
        "human_draft",
        "unsubmitted_envelope",
        "corrupted_submissions",
    ] {
        info.hold = Some(reason.into());
        for (prefix, letter) in [(1, 'A'), (26, 'Z')] {
            let text = visible(&status::render("s1", Some(&info), prefix, 80, 24));
            assert!(text.ends_with(&format!(" HELD {reason} · ^{letter} r ")));
        }
        for prefix in [0, 27, 255] {
            let text = visible(&status::render("s1", Some(&info), prefix, 80, 24));
            assert!(text.ends_with(&format!(" HELD {reason} ")));
            assert!(!text.contains('^'));
        }
    }
}

#[test]
fn partial_name_clipping_preserves_style_and_character_boundaries() {
    assert_eq!(
        status::render("éabc", None, 2, 5, 24),
        format!("{PRE}{}{BAR} \x1b[1méab\x1b[22m{POST}", "─".repeat(4)).as_bytes()
    );
}

#[test]
fn daemon_text_controls_are_replaced_before_layout() {
    let mut info = info("a\x1bb");
    assert_eq!(
        status::render("s1", Some(&info), 2, 40, 24),
        format!(
            "{PRE}{}{BAR} \x1b[1ma?b\x1b[22m {}{POST}",
            "─".repeat(39),
            " ".repeat(34)
        )
        .as_bytes()
    );
    assert_eq!(
        status::render("a\nb", None, 2, 6, 24),
        format!("{PRE}{}{BAR} \x1b[1ma?b\x1b[22m {POST}", "─".repeat(5)).as_bytes()
    );
    info.address = "a".into();
    info.hold = Some("x\n".into());
    assert_eq!(
        status::render("s1", Some(&info), 2, 10, 24),
        format!(
            "{PRE}{}{BAR}\x1b[0m\x1b[1;37;41m HELD x? {POST}",
            "─".repeat(9)
        )
        .as_bytes()
    );
}

#[test]
fn tiny_terminals_do_not_draw_a_bar() {
    assert_eq!(status::render("s1", None, 2, 40, 2), Vec::<u8>::new());
    assert_eq!(status::render("s1", None, 2, 1, 24), Vec::<u8>::new());
    assert_eq!(status::render("s1", None, 2, 0, 0), Vec::<u8>::new());
}

#[test]
fn separator_reserves_a_row_only_above_three_rows() {
    assert_eq!(
        status::render("s1", None, 2, 4, 4),
        "\x1b7\x1b[3;1H\x1b[0m\x1b[2K\x1b[38;5;240m───\x1b[0m\x1b[4;1H\x1b[0m\x1b[38;5;252;48;5;236m \x1b[1ms1\x1b[22m\x1b[0m\x1b8".as_bytes()
    );
    assert_eq!(
        status::render("s1", None, 2, 4, 3),
        "\x1b7\x1b[3;1H\x1b[0m\x1b[38;5;252;48;5;236m \x1b[1ms1\x1b[22m\x1b[0m\x1b8".as_bytes()
    );
    assert_eq!(status::render("s1", None, 2, 4, 2), Vec::<u8>::new());
}
