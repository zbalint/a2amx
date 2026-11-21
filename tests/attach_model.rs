use std::time::{Duration, Instant};

use a2amx::attach::model::{Attached, Geometry, Intent, Model, Step, Viewport};
use a2amx::emulator::Scroll;
use a2amx::harness::Harness;
use a2amx::messaging::TeamScope;
use a2amx::status::{
    TeamTarget, clear_status, render as render_status, render_team_line, render_team_rule,
};
use a2amx::viewer;
use a2amx::wire::{SessionSummary, StatusInfo, TeamMember};

fn visible_viewport(cols: u16, rows: u16) -> Viewport {
    Viewport {
        cols,
        rows,
        status_visible: true,
    }
}

fn test_model(
    in_team: bool,
    granted_top: u16,
    viewer: bool,
    viewer_after: Option<Duration>,
    t0: Instant,
) -> Model {
    Model::new(
        "s1".to_owned(),
        Geometry {
            viewport: visible_viewport(80, 24),
            in_team,
            granted_top,
        },
        0x02,
        viewer,
        viewer_after,
        t0,
    )
}

fn summary(id: &str) -> SessionSummary {
    SessionSummary {
        id: id.to_owned(),
        argv: vec!["sh".to_owned()],
        cols: 80,
        rows: 22,
        exit_code: None,
        attached: false,
        viewer: false,
        name: Some(id.to_owned()),
        team: None,
        address: format!("{id}@host-a"),
        pending: 0,
        held: false,
        hold_reason: None,
        quota: None,
        harness: Harness::Generic,
        cwd: None,
        confine: None,
        activity: None,
        uptime_secs: None,
        activity_secs: None,
        starting: false,
    }
}

fn team_status(team_name: &str, members: &[&str]) -> StatusInfo {
    StatusInfo {
        address: "s1@host-a".to_owned(),
        pending: 0,
        hold: None,
        harness: Some(Harness::Generic),
        activity: None,
        starting: false,
        confined: false,
        confine_auto: false,
        quota: None,
        team: Some(TeamScope {
            name: team_name.to_owned(),
            private: false,
            allow: Vec::new(),
            agents: Vec::new(),
        }),
        role: None,
        members: members
            .iter()
            .map(|id| TeamMember {
                id: (*id).to_owned(),
                name: (*id).to_owned(),
                activity: None,
                exited: false,
                held: false,
                starting: false,
            })
            .collect(),
        remote: None,
    }
}

#[test]
fn viewport_pty_rows_subtracts_status_and_top_rows() {
    let visible = Viewport {
        cols: 80,
        rows: 24,
        status_visible: true,
    };
    assert_eq!(visible.pty_rows(0), 22);
    assert_eq!(visible.pty_rows(2), 20);

    let hidden = Viewport {
        cols: 80,
        rows: 24,
        status_visible: false,
    };
    assert_eq!(hidden.pty_rows(2), 22);

    assert_eq!(
        Viewport {
            cols: 80,
            rows: 3,
            status_visible: true,
        }
        .pty_rows(0),
        2
    );
    assert_eq!(
        Viewport {
            cols: 80,
            rows: 2,
            status_visible: true,
        }
        .pty_rows(0),
        2
    );
    assert_eq!(
        Viewport {
            cols: 80,
            rows: 1,
            status_visible: true,
        }
        .pty_rows(0),
        1
    );
    assert_eq!(
        Viewport {
            cols: 80,
            rows: 4,
            status_visible: true,
        }
        .pty_rows(2),
        1
    );
}

#[test]
fn viewport_desired_top_follows_rows_visibility_and_team() {
    let view = |rows, status_visible| Viewport {
        cols: 80,
        rows,
        status_visible,
    };
    assert_eq!(view(24, true).desired_top(true), 2);
    assert_eq!(view(6, true).desired_top(true), 2);
    assert_eq!(view(5, true).desired_top(true), 1);
    assert_eq!(view(4, true).desired_top(true), 0);
    assert_eq!(view(24, false).desired_top(true), 0);
    assert_eq!(view(24, true).desired_top(false), 0);
}

#[test]
fn geometry_uses_the_granted_top_not_the_desired_one() {
    let viewport = Viewport {
        cols: 80,
        rows: 24,
        status_visible: true,
    };
    let refused = Geometry {
        viewport,
        in_team: true,
        granted_top: 0,
    };
    assert_eq!(refused.pty_rows(), 22);
    assert_eq!(refused.desired_top(), 2);

    let granted = Geometry {
        viewport,
        in_team: true,
        granted_top: 2,
    };
    assert_eq!(granted.pty_rows(), 20);
}

#[test]
fn step_plain_bytes_forwards_verbatim() {
    let t0 = Instant::now();
    let mut model = test_model(false, 0, false, None, t0);
    assert_eq!(
        model.step(b"abc", 0, t0),
        Step {
            consumed: 3,
            intents: vec![Intent::Forward(b"abc".to_vec())],
        }
    );
}

#[test]
fn step_detach_command_alone_and_with_leading_bytes() {
    let t0 = Instant::now();
    let mut model = test_model(false, 0, false, None, t0);
    assert_eq!(
        model.step(&[0x02, b'd'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::Detach],
        }
    );

    let mut model = test_model(false, 0, false, None, t0);
    assert_eq!(
        model.step(b"ab\x02dxyz", 0, t0),
        Step {
            consumed: 4,
            intents: vec![Intent::Forward(b"ab".to_vec()), Intent::Detach],
        }
    );
}

#[test]
fn step_open_picker_then_filter_key_in_same_read() {
    let t0 = Instant::now();
    let mut model = test_model(false, 0, false, None, t0);
    let read = b"\x02w/";
    assert_eq!(
        model.step(read, 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::OpenPicker],
        }
    );
    let opened = model.picker_opened(vec![summary("s1")]);
    assert_eq!(opened.len(), 1);
    assert!(matches!(opened[0], Intent::Output(_)));
    assert!(model.in_picker());

    let next = model.step(read, 2, t0);
    assert_eq!(next.consumed, 1);
    assert_eq!(next.intents.len(), 1);
    assert!(matches!(next.intents[0], Intent::Output(_)));
}

#[test]
fn step_scroll_mode_wheel_down_and_quit() {
    let t0 = Instant::now();
    let mut model = test_model(false, 0, false, None, t0);
    let read = b"\x02[\x1b[<65;10;5M";
    assert_eq!(
        model.step(read, 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::Redraw],
        }
    );
    assert_eq!(
        model.step(read, 2, t0),
        Step {
            consumed: 11,
            intents: vec![
                Intent::Scroll(Scroll::LineDown),
                Intent::Scroll(Scroll::LineDown),
                Intent::Scroll(Scroll::LineDown),
            ],
        }
    );
    assert_eq!(
        model.step(b"q", 0, t0),
        Step {
            consumed: 1,
            intents: vec![Intent::Scroll(Scroll::Bottom), Intent::Redraw],
        }
    );
    assert_eq!(
        model.step(b"x", 0, t0),
        Step {
            consumed: 1,
            intents: vec![Intent::Forward(b"x".to_vec())],
        }
    );
}

#[test]
fn step_wheel_up_with_session_mouse_off_enters_scroll_mode() {
    let t0 = Instant::now();
    let mut model = test_model(false, 0, false, None, t0);
    assert_eq!(
        model.step(b"\x1b[<64;10;5M", 0, t0),
        Step {
            consumed: 11,
            intents: vec![
                Intent::Redraw,
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
            ],
        }
    );
}

#[test]
fn step_session_mouse_pass_through_and_row_shifting() {
    let t0 = Instant::now();
    let mut model = test_model(false, 0, false, None, t0);
    let _ = model.on_output(b"\x1b[?1000h".to_vec(), t0);
    assert_eq!(
        model.step(b"\x1b[<0;10;5M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![Intent::Forward(b"\x1b[<0;10;5M".to_vec())],
        }
    );

    let mut team_model = test_model(true, 2, false, None, t0);
    let _ = team_model.on_output(b"\x1b[?1000h".to_vec(), t0);
    assert_eq!(
        team_model.step(b"\x1b[<0;10;5M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![Intent::Forward(b"\x1b[<0;10;3M".to_vec())],
        }
    );
}

#[test]
fn survivor_shift_uses_granted_top_not_desired_top() {
    let t0 = Instant::now();
    let mut model = Model::new(
        "s1".to_owned(),
        Geometry {
            viewport: visible_viewport(80, 4),
            in_team: true,
            granted_top: 2,
        },
        0x02,
        false,
        None,
        t0,
    );
    let _ = model.on_output(b"\x1b[?1000h".to_vec(), t0);
    assert_eq!(
        model.step(b"\x1b[<0;5;1M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(b"\x1b[<0;5;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
}

#[test]
fn survivor_attached_resets_session_mouse_status_and_granted_top() {
    let t0 = Instant::now();
    // (a) session_mouse reset
    let mut model = test_model(false, 0, false, None, t0);
    let _ = model.on_output(b"\x1b[?1000h".to_vec(), t0);
    assert_eq!(
        model.step(b"\x1b[<0;10;5M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![Intent::Forward(b"\x1b[<0;10;5M".to_vec())],
        }
    );
    assert_eq!(
        model.attached(
            "s2".to_owned(),
            Attached {
                in_team: false,
                granted_top: 0,
            },
        ),
        vec![Intent::Output(b"\x1b]2;a2amx: s2\x07".to_vec())]
    );
    assert_eq!(
        model.step(b"\x1b[<0;10;5M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![],
        }
    );

    // (b) status reset
    let mut model = test_model(true, 2, false, None, t0);
    let _ = model.on_status(team_status("t", &["s1", "s2"]), t0);
    assert_eq!(
        model.step(&[0x02, b'n'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::Switch {
                to: "s2".to_owned(),
            }],
        }
    );
    let _ = model.attached(
        "s1".to_owned(),
        Attached {
            in_team: true,
            granted_top: 2,
        },
    );
    assert_eq!(
        model.step(&[0x02, b'n'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![],
        }
    );

    // (c) granted_top reset
    let mut model = test_model(true, 2, false, None, t0);
    let _ = model.attached(
        "s1".to_owned(),
        Attached {
            in_team: false,
            granted_top: 0,
        },
    );
    let _ = model.on_output(b"\x1b[?1000h".to_vec(), t0);
    assert_eq!(
        model.step(b"\x1b[<0;10;1M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![Intent::Forward(b"\x1b[<0;10;1M".to_vec())],
        }
    );
}

#[test]
fn survivor_team_regions_cleared_on_attached_and_picker_opened() {
    let t0 = Instant::now();
    // Part 1: cleared on attached
    let mut model = test_model(true, 2, false, None, t0);
    let _ = model.on_status(team_status("t", &["s1", "s2"]), t0);
    assert_eq!(
        model.step(b"\x1b[<0;1;1M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(b"\x1b[<0;1;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![Intent::OpenPicker],
        }
    );
    let _ = model.attached(
        "s2".to_owned(),
        Attached {
            in_team: true,
            granted_top: 2,
        },
    );
    assert_eq!(
        model.step(b"\x1b[<0;1;1M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(b"\x1b[<0;1;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );

    // Part 2: cleared on picker_opened (Cancel restores Live without redrawing regions)
    let mut model = test_model(true, 2, false, None, t0);
    let _ = model.on_status(team_status("t", &["s1", "s2"]), t0);
    assert_eq!(
        model.step(&[0x02, b'w'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::OpenPicker],
        }
    );
    let _ = model.picker_opened(vec![summary("s1"), summary("s2")]);
    assert_eq!(
        model.step(b"\x1b", 0, t0),
        Step {
            consumed: 1,
            intents: vec![Intent::Redraw],
        }
    );
    assert!(!model.in_picker());
    // Column 17 is inside member s2's region before picker_opened cleared team_regions.
    assert_eq!(
        model.step(b"\x1b[<0;17;1M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(b"\x1b[<0;17;1m", 0, t0),
        Step {
            consumed: 10,
            intents: vec![],
        }
    );
}

#[test]
fn survivor_scroll_entry_keeps_parser_while_already_scrolling() {
    let t0 = Instant::now();
    let mut model = test_model(false, 0, false, None, t0);
    assert_eq!(
        model.step(&[0x02, b'['], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::Redraw],
        }
    );
    assert_eq!(
        model.step(b"\x1b", 0, t0),
        Step {
            consumed: 1,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(b"\x1b[<64;10;5M", 0, t0),
        Step {
            consumed: 11,
            intents: vec![
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
            ],
        }
    );
    assert_eq!(
        model.step(b"[A", 0, t0),
        Step {
            consumed: 1,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(b"[A", 1, t0),
        Step {
            consumed: 1,
            intents: vec![Intent::Scroll(Scroll::LineUp)],
        }
    );
}

#[test]
fn viewer_blocks_batches_and_ignores_focus_report_for_idle_clock() {
    let t0 = Instant::now();
    let mut model = test_model(false, 0, true, None, t0);
    let expected_marker = viewer::marker(80, 24);
    let expected_overlay = viewer::overlay(80, 22, 0, 0x02);
    assert_eq!(
        model.step(b"a", 0, t0),
        Step {
            consumed: 1,
            intents: vec![
                Intent::Output(expected_marker.clone()),
                Intent::Output(expected_overlay.clone()),
            ],
        }
    );

    let read = b"a\x1b[<0;10;5Mb";
    assert_eq!(
        model.step(read, 0, t0),
        Step {
            consumed: 1,
            intents: vec![
                Intent::Output(expected_marker.clone()),
                Intent::Output(expected_overlay.clone()),
            ],
        }
    );
    assert_eq!(
        model.step(read, 1, t0),
        Step {
            consumed: 10,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(read, 11, t0),
        Step {
            consumed: 1,
            intents: vec![
                Intent::Output(expected_marker.clone()),
                Intent::Output(expected_overlay),
            ],
        }
    );
    assert_eq!(
        model.step(b"\x1b[I", 0, t0),
        Step {
            consumed: 3,
            intents: vec![],
        }
    );

    let mut idle_model = test_model(false, 0, false, Some(Duration::from_secs(1)), t0);
    assert_eq!(
        idle_model.step(b"\x1b[I", 0, t0 + Duration::from_millis(900)),
        Step {
            consumed: 3,
            intents: vec![Intent::Forward(b"\x1b[I".to_vec())],
        }
    );
    assert_eq!(
        idle_model.on_tick(t0 + Duration::from_secs(1)),
        vec![
            Intent::SetViewer { viewer: true },
            Intent::Output(render_status("s1", None, 0x02, 80, 24)),
            Intent::Output(expected_marker),
        ]
    );

    let mut active_model = test_model(false, 0, false, Some(Duration::from_secs(1)), t0);
    assert_eq!(
        active_model.step(b"a", 0, t0 + Duration::from_millis(900)),
        Step {
            consumed: 1,
            intents: vec![Intent::Forward(b"a".to_vec())],
        }
    );
    assert_eq!(active_model.on_tick(t0 + Duration::from_secs(1)), vec![]);
}

#[test]
fn resize_sends_pty_rows() {
    let t0 = Instant::now();
    let mut model = test_model(false, 0, false, None, t0);
    assert_eq!(
        model.resize(100, 30, t0),
        vec![Intent::Resize {
            cols: 100,
            rows: 28,
        }]
    );
}

#[test]
fn status_toggle_with_and_without_team_rows() {
    let t0 = Instant::now();
    let mut team_model = test_model(true, 2, false, None, t0);
    assert_eq!(
        team_model.step(&[0x02, b's'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![
                Intent::Output(render_team_line("s1", "", &[], 80).bytes),
                Intent::Output(render_team_rule(80, false)),
                Intent::Output(clear_status(80, 24)),
                Intent::Switch {
                    to: "s1".to_owned(),
                },
            ],
        }
    );

    let mut plain_model = test_model(false, 0, false, None, t0);
    assert_eq!(
        plain_model.step(&[0x02, b's'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![
                Intent::Output(clear_status(80, 24)),
                Intent::Resize { cols: 80, rows: 24 },
            ],
        }
    );
}

#[test]
fn on_output_reemits_mouse_capture_only_after_disable() {
    let t0 = Instant::now();
    let mut model = test_model(false, 0, false, None, t0);
    let enable = model.on_output(b"\x1b[?1000h".to_vec(), t0);
    assert_eq!(
        enable,
        vec![
            Intent::Output(b"\x1b[?1000h".to_vec()),
            Intent::Output(render_status("s1", None, 0x02, 80, 24)),
        ]
    );

    let disable = model.on_output(b"\x1b[?1000l".to_vec(), t0);
    assert_eq!(
        disable,
        vec![
            Intent::Output(b"\x1b[?1000l".to_vec()),
            Intent::Output(b"\x1b[?1000h\x1b[?1006h".to_vec()),
            Intent::Output(render_status("s1", None, 0x02, 80, 24)),
        ]
    );
}

#[test]
fn next_member_skips_exited_members() {
    let t0 = Instant::now();
    let mut model = test_model(true, 2, false, None, t0);
    let mut status = team_status("t", &["s1", "s2", "s3"]);
    status.members[1].exited = true;
    let _ = model.on_status(status, t0);

    assert_eq!(
        model.step(&[0x02, b'n'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::Switch {
                to: "s3".to_owned(),
            }],
        }
    );
}

#[test]
fn prev_member_moves_backward_and_wraps_to_last() {
    let t0 = Instant::now();
    let mut model_s2 = Model::new(
        "s2".to_owned(),
        Geometry {
            viewport: visible_viewport(80, 24),
            in_team: true,
            granted_top: 2,
        },
        0x02,
        false,
        None,
        t0,
    );
    let _ = model_s2.on_status(team_status("t", &["s1", "s2", "s3"]), t0);
    assert_eq!(
        model_s2.step(&[0x02, b'p'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::Switch {
                to: "s1".to_owned(),
            }],
        }
    );

    let mut model_s1 = test_model(true, 2, false, None, t0);
    let _ = model_s1.on_status(team_status("t", &["s1", "s2", "s3"]), t0);
    assert_eq!(
        model_s1.step(&[0x02, b'p'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::Switch {
                to: "s3".to_owned(),
            }],
        }
    );
}

#[test]
fn team_click_column_bounds_are_inclusive() {
    let t0 = Instant::now();
    let status = team_status("t", &["s1"]);
    let line = render_team_line("s1", "t", &status.members, 80);
    let picker_region = line
        .regions
        .iter()
        .find(|region| region.target == TeamTarget::Picker)
        .expect("picker region");
    assert_eq!(picker_region.first_col, 1);
    assert_eq!(picker_region.last_col, 10);

    let click_at = |model: &mut Model, col: u16| {
        let press = format!("\x1b[<0;{col};1M").into_bytes();
        let release = format!("\x1b[<0;{col};1m").into_bytes();
        assert_eq!(model.step(&press, 0, t0).intents, vec![]);
        model.step(&release, 0, t0).intents
    };

    for col in [picker_region.first_col, picker_region.last_col] {
        let mut model = test_model(true, 2, false, None, t0);
        let _ = model.on_status(status.clone(), t0);
        assert_eq!(click_at(&mut model, col), vec![Intent::OpenPicker]);
    }

    for col in [picker_region.first_col - 1, picker_region.last_col + 1] {
        let mut model = test_model(true, 2, false, None, t0);
        let _ = model.on_status(status.clone(), t0);
        assert_eq!(click_at(&mut model, col), vec![]);
    }
}

#[test]
fn switch_failed_preserves_team_regions() {
    let t0 = Instant::now();
    let mut model = test_model(true, 2, false, None, t0);
    let _ = model.on_status(team_status("t", &["s1", "s2"]), t0);
    assert_eq!(
        model.switch_failed(
            "boom".to_owned(),
            Attached {
                in_team: true,
                granted_top: 2,
            },
        ),
        vec![]
    );
    assert_eq!(
        model.step(b"\x1b[<0;1;1M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(b"\x1b[<0;1;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![Intent::OpenPicker],
        }
    );
}

#[test]
fn picker_footer_shows_on_switch_failed_and_clears_on_next_byte() {
    let t0 = Instant::now();
    let mut model = test_model(false, 0, false, None, t0);
    assert_eq!(
        model.step(&[0x02, b'w'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::OpenPicker],
        }
    );
    let sessions = vec![summary("s1"), summary("s2")];
    let _ = model.picker_opened(sessions.clone());
    assert_eq!(
        model.switch_failed(
            "boom".to_owned(),
            Attached {
                in_team: false,
                granted_top: 0,
            },
        ),
        vec![Intent::RefreshPicker]
    );

    let refreshed = model.picker_sessions(sessions);
    assert_eq!(refreshed.len(), 1);
    let Intent::Output(refreshed_bytes) = &refreshed[0] else {
        panic!("expected Output intent from picker_sessions");
    };
    assert!(
        String::from_utf8_lossy(refreshed_bytes).contains("boom"),
        "expected footer error in refreshed render"
    );

    let next = model.step(b"j", 0, t0);
    assert_eq!(next.consumed, 1);
    assert_eq!(next.intents.len(), 2);
    let Intent::Output(cleared_bytes) = &next.intents[0] else {
        panic!("expected first intent to clear footer");
    };
    assert!(
        !String::from_utf8_lossy(cleared_bytes).contains("boom"),
        "expected footer cleared on next picker byte"
    );
}
