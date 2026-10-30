use a2amx::harness::Harness;
use a2amx::status;
use a2amx::wire::{Activity, QuotaInfo, StatusInfo, TeamMember};

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
        confined: false,
        starting: false,
        quota: None,
        team: None,
        role: None,
        members: vec![],
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
fn confined_status_shows_dim_confined_tag() {
    let mut status = info("agent-plan@host-a");
    status.confined = true;
    let rendered = status::render("agent-plan", Some(&status), 2, 80, 24);
    let text = String::from_utf8(rendered).expect("utf8 status");
    assert!(text.contains("\x1b[2m[confined]\x1b[22m"));
}

#[test]
fn status_shows_sanitized_role_and_private_team_before_harness() {
    let mut status = info("agent-plan@host-a");
    status.role = Some("architect\nunsafe".into());
    status.team = Some(a2amx::messaging::TeamScope {
        name: "demo\r\nteam".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    });
    status.harness = Some(Harness::Claude);
    let text = visible(&status::render(
        "agent-plan@host-a",
        Some(&status),
        2,
        160,
        24,
    ));
    assert!(text.contains("role architect?unsafe"));
    assert!(text.contains("team demo??team (private)"));
    assert!(text.contains("role architect?unsafe │ team demo??team (private) │ claude"));
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
fn activity_dot_states_cover_starting_resetting_and_precedence() {
    let mut info = info("a");
    info.activity = Some(Activity::Busy);
    info.starting = true;
    let starting = status::render("s1", Some(&info), 2, 40, 24);
    assert!(
        starting
            .windows(b"\x1b[38;5;221m".len())
            .any(|window| window == b"\x1b[38;5;221m")
    );
    assert!(visible(&starting).starts_with(" a │ ● starting "));

    info.starting = false;
    let resetting = status::render("s1", Some(&info), 2, 40, 24);
    assert!(
        resetting
            .windows(b"\x1b[38;5;209m".len())
            .any(|window| window == b"\x1b[38;5;209m")
    );
    assert!(visible(&resetting).starts_with(" a │ ● resetting "));
}

#[test]
fn team_line_dot_states_cover_held_starting_exited_and_unknown() {
    let members = [
        TeamMember {
            id: "held".into(),
            name: "held".into(),
            activity: Some(Activity::Working),
            exited: false,
            held: true,
            starting: true,
        },
        TeamMember {
            id: "starting".into(),
            name: "starting".into(),
            activity: Some(Activity::Busy),
            exited: false,
            held: false,
            starting: true,
        },
        TeamMember {
            id: "exited".into(),
            name: "exited".into(),
            activity: Some(Activity::Busy),
            exited: true,
            held: true,
            starting: true,
        },
        TeamMember {
            id: "unknown".into(),
            name: "unknown".into(),
            activity: None,
            exited: false,
            held: true,
            starting: true,
        },
    ];
    let bytes = status::render_team_line("other", "dev", &members, 200).bytes;
    assert!(String::from_utf8_lossy(&bytes).contains("\x1b[38;5;203m●\x1b[38;5;252m held"));
    assert!(String::from_utf8_lossy(&bytes).contains("\x1b[38;5;221m●\x1b[38;5;252m starting"));
    assert!(String::from_utf8_lossy(&bytes).contains("\x1b[38;5;244m●\x1b[38;5;252m exited"));
    assert!(String::from_utf8_lossy(&bytes).contains("\x1b[38;5;244m●\x1b[38;5;252m unknown"));
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
    assert_eq!(status::render("s1", Some(&info), 2, 40, 24), format!("{PRE}{}{BAR} \x1b[1ma\x1b[22m \x1b[38;5;240m│\x1b[38;5;252m \x1b[38;5;75m●\x1b[38;5;252m working {}{POST}", "─".repeat(39), " ".repeat(24)).as_bytes());
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

#[test]
fn team_rule_draws_clears_and_skips_narrow_terminals() {
    assert_eq!(
        status::render_team_rule(10, true),
        "\x1b7\x1b[2;1H\x1b[0m\x1b[2K\x1b[38;5;240m─────────\x1b[0m\x1b8".as_bytes()
    );
    assert_eq!(
        status::render_team_rule(10, false),
        b"\x1b7\x1b[2;1H\x1b[0m\x1b[2K\x1b8".to_vec()
    );
    assert!(status::render_team_rule(1, true).is_empty());
}

fn team_member(id: &str, name: &str, activity: Option<Activity>, exited: bool) -> TeamMember {
    TeamMember {
        id: id.into(),
        name: name.into(),
        activity,
        exited,
        held: false,
        starting: false,
    }
}

#[test]
fn team_line_matches_the_locked_worked_example() {
    let line = status::render_team_line(
        "a",
        "dev",
        &[
            team_member("a", "alpha", Some(Activity::Idle), false),
            team_member("b", "beta", Some(Activity::Working), false),
        ],
        40,
    );
    assert_eq!(
        line.bytes,
        [
            b"\x1b7\x1b[1;1H\x1b[0m\x1b[2K\x1b[38;5;252;48;5;236m team dev \x1b[38;5;240m\xe2\x94\x82\x1b[38;5;252m \x1b[38;5;78m\xe2\x97\x8f\x1b[38;5;252m \x1b[1malpha\x1b[22m  \x1b[38;5;75m\xe2\x97\x8f\x1b[38;5;252m beta  ".as_slice(),
            b"          ",
            b"\x1b[0m\x1b8",
        ]
        .concat()
    );
    assert_eq!(
        line.regions,
        vec![
            status::TeamRegion {
                first_col: 1,
                last_col: 12,
                target: status::TeamTarget::Picker,
            },
            status::TeamRegion {
                first_col: 13,
                last_col: 21,
                target: status::TeamTarget::Session("a".into()),
            },
            status::TeamRegion {
                first_col: 22,
                last_col: 29,
                target: status::TeamTarget::Session("b".into()),
            },
        ]
    );
}

#[test]
fn team_line_truncates_names_and_preserves_current_member_on_overflow() {
    let members = [
        team_member("a", "alpha", Some(Activity::Idle), false),
        team_member("b", "beta", Some(Activity::Working), false),
        team_member("c", "gamma", Some(Activity::Busy), false),
    ];
    let truncated = status::render_team_line(
        "a",
        "a very long team name",
        &[team_member(
            "a",
            "a very long member name",
            Some(Activity::Idle),
            false,
        )],
        80,
    );
    let truncated_text = String::from_utf8_lossy(&truncated.bytes);
    assert!(truncated_text.contains("team a very long tea…"));
    assert!(truncated_text.contains("a very long mem…"));
    let exact = status::render_team_line(
        "a",
        "dev",
        &[team_member(
            "a",
            "1234567890123456",
            Some(Activity::Idle),
            false,
        )],
        80,
    );
    let exact_text = String::from_utf8_lossy(&exact.bytes);
    assert!(exact_text.contains("1234567890123456"));
    assert!(!exact_text.contains("1234567890123456…"));
    let seventeen = status::render_team_line(
        "a",
        "dev",
        &[team_member(
            "a",
            "12345678901234567",
            Some(Activity::Idle),
            false,
        )],
        80,
    );
    let seventeen_text = String::from_utf8_lossy(&seventeen.bytes);
    assert!(seventeen_text.contains("123456789012345…"));
    assert!(!seventeen_text.contains("12345678901234567"));

    let overflow = status::render_team_line("b", "dev", &members, 30);
    assert!(
        overflow
            .regions
            .iter()
            .any(|region| region.target == status::TeamTarget::Session("b".into()))
    );
    assert!(
        overflow
            .regions
            .iter()
            .any(|region| region.target == status::TeamTarget::Picker)
    );
    assert!(String::from_utf8_lossy(&overflow.bytes).contains(" +2 "));
}

#[test]
fn team_line_clears_empty_members_and_rejects_narrow_columns() {
    assert_eq!(
        status::render_team_line("a", "dev", &[], 4).bytes,
        b"\x1b7\x1b[1;1H\x1b[0m\x1b[2K\x1b[38;5;252;48;5;236m   \x1b[0m\x1b8".to_vec()
    );
    assert!(
        status::render_team_line("a", "dev", &[], 1)
            .bytes
            .is_empty()
    );
}
