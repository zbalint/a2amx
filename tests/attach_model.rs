use std::time::{Duration, Instant};

use a2amx::attach::model::{Attached, Geometry, Intent, Model, ModelConfig, Step, Viewport};
use a2amx::emulator::Scroll;
use a2amx::harness::Harness;
use a2amx::messaging::TeamScope;
use a2amx::status::{
    StatusBar, TeamTarget, clear_status, render as render_status, render_team_line,
    render_team_rule,
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
        ModelConfig {
            session: "s1".to_owned(),
            geometry: Geometry {
                viewport: visible_viewport(80, 24),
                in_team,
                granted_top,
            },
            prefix: 0x02,
            viewer,
            viewer_after,
        },
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
        ModelConfig {
            session: "s1".to_owned(),
            geometry: Geometry {
                viewport: visible_viewport(80, 4),
                in_team: true,
                granted_top: 2,
            },
            prefix: 0x02,
            viewer: false,
            viewer_after: None,
        },
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
            Intent::Output(render_status(&StatusBar::new("s1", None, 0x02), 80, 24)),
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
            Intent::Output(render_status(&StatusBar::new("s1", None, 0x02), 80, 24)),
        ]
    );

    let disable = model.on_output(b"\x1b[?1000l".to_vec(), t0);
    assert_eq!(
        disable,
        vec![
            Intent::Output(b"\x1b[?1000l".to_vec()),
            Intent::Output(b"\x1b[?1000h\x1b[?1006h".to_vec()),
            Intent::Output(render_status(&StatusBar::new("s1", None, 0x02), 80, 24)),
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
        ModelConfig {
            session: "s2".to_owned(),
            geometry: Geometry {
                viewport: visible_viewport(80, 24),
                in_team: true,
                granted_top: 2,
            },
            prefix: 0x02,
            viewer: false,
            viewer_after: None,
        },
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
    let opened = model.picker_opened(sessions.clone());
    assert_eq!(opened.len(), 1);
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
    assert_eq!(next.intents[0], opened[0]);
}

#[test]
fn step_guards_pass_through_segmentation_and_mode_dispatch() {
    let t0 = Instant::now();
    let after = Some(Duration::from_secs(10));

    // Empty read or offset >= read.len() returns consumed=0 and does not reset viewer_after.
    let mut model = test_model(false, 0, false, after, t0);
    assert_eq!(
        model.step(b"", 0, t0 + Duration::from_secs(5)),
        Step {
            consumed: 0,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(b"abc", 3, t0 + Duration::from_secs(5)),
        Step {
            consumed: 0,
            intents: vec![],
        }
    );
    assert_eq!(
        model.on_tick(t0 + Duration::from_secs(10)),
        vec![
            Intent::SetViewer { viewer: true },
            Intent::Output(render_status(&StatusBar::new("s1", None, 0x02), 80, 24)),
            Intent::Output(viewer::marker(80, 24)),
        ]
    );

    // Focus report at offset 0 and non-zero offset do NOT call viewer.own_input.
    let mut model = test_model(false, 0, false, after, t0);
    assert_eq!(
        model.step(b"\x1b[I", 0, t0 + Duration::from_secs(5)),
        Step {
            consumed: 3,
            intents: vec![Intent::Forward(b"\x1b[I".to_vec())],
        }
    );
    assert_eq!(
        model.step(b"ab", 1, t0 + Duration::from_secs(5)),
        Step {
            consumed: 1,
            intents: vec![Intent::Forward(b"b".to_vec())],
        }
    );
    assert_eq!(
        model.on_tick(t0 + Duration::from_secs(10)),
        vec![
            Intent::SetViewer { viewer: true },
            Intent::Output(render_status(&StatusBar::new("s1", None, 0x02), 80, 24)),
            Intent::Output(viewer::marker(80, 24)),
        ]
    );

    // Non-focus input at offset 0 resets viewer_after timer.
    let mut model = test_model(false, 0, false, after, t0);
    assert_eq!(
        model.step(b"a", 0, t0 + Duration::from_secs(5)),
        Step {
            consumed: 1,
            intents: vec![Intent::Forward(b"a".to_vec())],
        }
    );
    assert_eq!(model.on_tick(t0 + Duration::from_secs(10)), vec![]);

    // pass_through: live + session_mouse + !viewer + granted_top == 0 forwards raw mouse report.
    let mut model = test_model(false, 0, false, None, t0);
    let _ = model.on_output(b"\x1b[?1000h".to_vec(), t0);
    assert_eq!(
        model.step(b"\x1b[<64;1;1M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![Intent::Forward(b"\x1b[<64;1;1M".to_vec())],
        }
    );

    // With viewer=true and granted_top=0, pass_through is false: wheel 64 enters scroll mode.
    let mut viewer_model = test_model(false, 0, true, None, t0);
    let _ = viewer_model.on_output(b"\x1b[?1000h".to_vec(), t0);
    assert_eq!(
        viewer_model.step(b"\x1b[<64;1;1M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![
                Intent::Redraw,
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
            ],
        }
    );

    // segment_end stops at a later mouse report (start > offset).
    let mut seg_model = test_model(false, 0, false, None, t0);
    assert_eq!(
        seg_model.step(b"ab\x1b[<64;1;1M", 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::Forward(b"ab".to_vec())],
        }
    );
    assert_eq!(
        seg_model.step(b"ab\x1b[<64;1;1M", 2, t0),
        Step {
            consumed: 10,
            intents: vec![
                Intent::Redraw,
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
            ],
        }
    );

    // Scroll mode consumes 1 byte per step.
    assert_eq!(
        seg_model.step(b"qk", 0, t0),
        Step {
            consumed: 1,
            intents: vec![Intent::Scroll(Scroll::Bottom), Intent::Redraw],
        }
    );

    // Picker mode: mouse report at offset is consumed whole with no intents;
    // escape_alone uses full read == [0x1b] (so b"x\x1b" at offset 1 does NOT cancel).
    let mut picker_model = test_model(false, 0, false, None, t0);
    assert_eq!(
        picker_model.step(&[0x02, b'w'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::OpenPicker],
        }
    );
    let _ = picker_model.picker_opened(vec![summary("s1"), summary("s2")]);
    assert_eq!(
        picker_model.step(b"\x1b[<64;1;1M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![],
        }
    );
    let non_alone_esc = picker_model.step(b"x\x1b", 1, t0);
    assert_eq!(non_alone_esc.consumed, 1);
    assert!(picker_model.in_picker());
    assert_eq!(
        picker_model.step(b"\x1b", 0, t0),
        Step {
            consumed: 1,
            intents: vec![Intent::Redraw],
        }
    );
    assert!(!picker_model.in_picker());
}

#[test]
fn mouse_report_and_wheel_branches_are_exact() {
    let t0 = Instant::now();
    let mut model = test_model(true, 2, false, None, t0);
    let _ = model.on_status(team_status("t", &["s1", "s2"]), t0);
    let _ = model.on_output(b"\x1b[?1000h".to_vec(), t0);

    // Claimed press row 2 stores Rule; release on row 1 does nothing and clears press.
    assert_eq!(
        model.step(b"\x1b[<0;1;2M", 0, t0),
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
    // Subsequent release on row 1 also does nothing (press was cleared).
    assert_eq!(
        model.step(b"\x1b[<0;1;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );

    // Claimed non-release with button != 0 (including wheel 64/65 on row 1/2) does nothing
    // and leaves existing Team press intact.
    assert_eq!(
        model.step(b"\x1b[<0;1;1M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(b"\x1b[<64;1;1M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(b"\x1b[<1;1;2M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );

    // Grid press (row 3, button 0) overwrites Team press with Grid and forwards with session_mouse.
    assert_eq!(
        model.step(b"\x1b[<0;5;3M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![Intent::Forward(b"\x1b[<0;5;1M".to_vec())],
        }
    );
    // Claimed release on row 1 with Grid press + session_mouse forwards with row=1 (not team_click!).
    assert_eq!(
        model.step(b"\x1b[<0;5;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![Intent::Forward(b"\x1b[<0;5;1m".to_vec())],
        }
    );

    // Claimed press on row 1 followed by Grid release ignores Team press and clears it.
    assert_eq!(
        model.step(b"\x1b[<0;1;1M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(b"\x1b[<0;5;3m", 0, t0),
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

    // Grid release with None press + session_mouse forwards; without session_mouse does nothing.
    assert_eq!(
        model.step(b"\x1b[<0;5;3m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![Intent::Forward(b"\x1b[<0;5;1m".to_vec())],
        }
    );
    let mut no_mouse = test_model(true, 2, false, None, t0);
    assert_eq!(
        no_mouse.step(b"\x1b[<0;5;3m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    // Without session_mouse, wheel 65 in Live mode does nothing; wheel 64 enters Scroll + 3 LineUp;
    // wheel 64 again pushes 3 LineUp without Redraw; wheel 65 in Scroll pushes 3 LineDown; button 0 nothing.
    assert_eq!(
        no_mouse.step(b"\x1b[<65;5;3M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![],
        }
    );
    assert_eq!(
        no_mouse.step(b"\x1b[<64;5;3M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![
                Intent::Redraw,
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
            ],
        }
    );
    assert_eq!(
        no_mouse.step(b"\x1b[<64;5;3M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
            ],
        }
    );
    assert_eq!(
        no_mouse.step(b"\x1b[<65;5;3M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![
                Intent::Scroll(Scroll::LineDown),
                Intent::Scroll(Scroll::LineDown),
                Intent::Scroll(Scroll::LineDown),
            ],
        }
    );
    assert_eq!(
        no_mouse.step(b"\x1b[<0;5;3M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
}

#[test]
fn scan_session_mouse_modes_and_picker_update() {
    let t0 = Instant::now();
    let mut model = test_model(false, 0, false, None, t0);

    // Modes 1002 and 1003 enable/disable, 1001/1006/25 ignored, truncated ESC[?1000 safe.
    for enable in [b"\x1b[?1002h".as_slice(), b"\x1b[?1003h".as_slice()] {
        assert_eq!(
            model.on_output(enable.to_vec(), t0),
            vec![
                Intent::Output(enable.to_vec()),
                Intent::Output(render_status(&StatusBar::new("s1", None, 0x02), 80, 24)),
            ]
        );
    }
    // Last toggle wins: 1002h followed by 1003l disables and re-emits MOUSE_CAPTURE.
    assert_eq!(
        model.on_output(b"\x1b[?1002h\x1b[?1003l".to_vec(), t0),
        vec![
            Intent::Output(b"\x1b[?1002h\x1b[?1003l".to_vec()),
            Intent::Output(b"\x1b[?1000h\x1b[?1006h".to_vec()),
            Intent::Output(render_status(&StatusBar::new("s1", None, 0x02), 80, 24)),
        ]
    );
    // Near miss 1001, 1006, 25, and truncated ESC[?1000 do NOT emit MOUSE_CAPTURE.
    for ignored in [
        b"\x1b[?1001l".as_slice(),
        b"\x1b[?1006l".as_slice(),
        b"\x1b[?25l".as_slice(),
        b"\x1b[?1000".as_slice(),
    ] {
        assert_eq!(
            model.on_output(ignored.to_vec(), t0),
            vec![
                Intent::Output(ignored.to_vec()),
                Intent::Output(render_status(&StatusBar::new("s1", None, 0x02), 80, 24)),
            ]
        );
    }

    // In picker mode, on_output returns vec![] but still updates session_mouse.
    let _ = model.step(&[0x02, b'w'], 0, t0);
    let _ = model.picker_opened(vec![summary("s1")]);
    assert_eq!(model.on_output(b"\x1b[?1000h".to_vec(), t0), vec![]);
    assert_eq!(
        model.step(b"\x1b", 0, t0),
        Step {
            consumed: 1,
            intents: vec![Intent::Redraw],
        }
    );
    assert_eq!(
        model.step(b"\x1b[<0;1;1M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![Intent::Forward(b"\x1b[<0;1;1M".to_vec())],
        }
    );
}

#[test]
fn select_member_clears_press_and_picker_attach_own_vs_other() {
    let t0 = Instant::now();
    let mut model = test_model(true, 2, false, None, t0);
    let _ = model.on_status(team_status("t", &["s1", "s2"]), t0);

    // Team press followed by keyboard switch clears mouse_press.
    assert_eq!(
        model.step(b"\x1b[<0;1;1M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    assert_eq!(
        model.step(&[0x02, b'n'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::Switch {
                to: "s2".to_owned(),
            }],
        }
    );
    let _ = model.switch_failed(
        "err".to_owned(),
        Attached {
            in_team: true,
            granted_top: 2,
        },
    );
    assert_eq!(
        model.step(b"\x1b[<0;1;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );

    // Viewer decorations: nothing when not live (scroll mode) or status hidden.
    let mut viewer = test_model(false, 0, true, None, t0);
    assert_eq!(
        viewer.step(&[0x02, b'['], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::Redraw],
        }
    );
    assert_eq!(viewer.on_tick(t0 + Duration::from_secs(1)), vec![]);

    // Picker Attach(own session) -> Redraw and back to Live; Attach(other) -> Switch.
    let mut picker = test_model(false, 0, false, None, t0);
    let _ = picker.step(&[0x02, b'w'], 0, t0);
    let _ = picker.picker_opened(vec![summary("s1"), summary("s2")]);
    assert_eq!(
        picker.step(b"\r", 0, t0),
        Step {
            consumed: 1,
            intents: vec![Intent::Redraw],
        }
    );
    assert!(!picker.in_picker());

    let _ = picker.step(&[0x02, b'w'], 0, t0);
    let _ = picker.picker_opened(vec![summary("s1"), summary("s2")]);
    let _ = picker.step(b"j", 0, t0);
    assert_eq!(
        picker.step(b"\r", 0, t0),
        Step {
            consumed: 1,
            intents: vec![Intent::Switch {
                to: "s2".to_owned(),
            }],
        }
    );
}

#[test]
fn tester_survivor_guards_and_mouse_edge_cases() {
    let t0 = Instant::now();

    // 1. granted_top > 0 && live: in Scroll mode with granted_top == 2, wheel press 64 uses handle_mouse_button;
    // with granted_top == 0 in Live mode, wheel release 64 also uses handle_mouse_button.
    let mut scroll_team = test_model(true, 2, false, None, t0);
    assert_eq!(
        scroll_team.step(&[0x02, b'['], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::Redraw],
        }
    );
    assert_eq!(
        scroll_team.step(b"\x1b[<64;1;1M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
            ],
        }
    );
    let mut top0_live = test_model(false, 0, false, None, t0);
    assert_eq!(
        top0_live.step(b"\x1b[<64;1;1m", 0, t0),
        Step {
            consumed: 10,
            intents: vec![
                Intent::Redraw,
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
                Intent::Scroll(Scroll::LineUp),
            ],
        }
    );

    // 2. viewer_decorations_visible without status_visible: on_tick emits only SetViewer.
    let mut hidden_status = Model::new(
        ModelConfig {
            session: "s1".to_owned(),
            geometry: Geometry {
                viewport: Viewport {
                    cols: 80,
                    rows: 24,
                    status_visible: false,
                },
                in_team: false,
                granted_top: 0,
            },
            prefix: 0x02,
            viewer: false,
            viewer_after: Some(Duration::from_secs(1)),
        },
        t0,
    );
    assert_eq!(
        hidden_status.on_tick(t0 + Duration::from_secs(2)),
        vec![Intent::SetViewer { viewer: true }]
    );

    // 3. viewer_decorations_visible without live (picker open): on_tick emits only SetViewer.
    let mut picker_viewer = test_model(false, 0, false, Some(Duration::from_secs(1)), t0);
    assert_eq!(
        picker_viewer.step(&[0x02, b'w'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::OpenPicker],
        }
    );
    let _ = picker_viewer.picker_opened(vec![summary("s1")]);
    assert_eq!(
        picker_viewer.on_tick(t0 + Duration::from_secs(2)),
        vec![Intent::SetViewer { viewer: true }]
    );

    // 4. claimed_release take() clears press so second release gives [].
    let mut team_click_twice = test_model(true, 2, false, None, t0);
    let _ = team_click_twice.on_status(team_status("t", &["s1", "s2"]), t0);
    assert_eq!(
        team_click_twice.step(b"\x1b[<0;1;1M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    assert_eq!(
        team_click_twice.step(b"\x1b[<0;1;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![Intent::OpenPicker],
        }
    );
    assert_eq!(
        team_click_twice.step(b"\x1b[<0;1;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );

    // 6. claimed_release session_mouse requires !viewer.
    let mut viewer_team = test_model(true, 2, true, None, t0);
    let _ = viewer_team.on_output(b"\x1b[?1000h".to_vec(), t0);
    assert_eq!(
        viewer_team.step(b"\x1b[<0;5;3M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    assert_eq!(
        viewer_team.step(b"\x1b[<0;5;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );

    // 7. mouse_mode_toggle requires ESC [ ? prefix.
    let mut prefix_check = test_model(false, 0, false, None, t0);
    assert_eq!(
        prefix_check.on_output(b"abc1000h".to_vec(), t0),
        vec![
            Intent::Output(b"abc1000h".to_vec()),
            Intent::Output(render_status(&StatusBar::new("s1", None, 0x02), 80, 24)),
        ]
    );
    assert_eq!(
        prefix_check.step(b"\x1b[<0;5;3M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );

    // 8. scan_session_mouse stride is 1 (odd offset toggle still recognized).
    let mut odd_offset = test_model(false, 0, false, None, t0);
    assert_eq!(
        odd_offset.on_output(b"x\x1b[?1000h".to_vec(), t0),
        vec![
            Intent::Output(b"x\x1b[?1000h".to_vec()),
            Intent::Output(render_status(&StatusBar::new("s1", None, 0x02), 80, 24)),
        ]
    );
    assert_eq!(
        odd_offset.step(b"\x1b[<0;5;3M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![Intent::Forward(b"\x1b[<0;5;3M".to_vec())],
        }
    );

    // 9. Picker Attach(other) clears mouse_press.
    let mut attach_other = test_model(true, 2, false, None, t0);
    let _ = attach_other.on_status(team_status("t", &["s1", "s2"]), t0);
    assert_eq!(
        attach_other.step(b"\x1b[<0;1;1M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    assert_eq!(
        attach_other.step(&[0x02, b'w'], 0, t0),
        Step {
            consumed: 2,
            intents: vec![Intent::OpenPicker],
        }
    );
    let _ = attach_other.picker_opened(vec![summary("s1"), summary("s2")]);
    let _ = attach_other.step(b"j", 0, t0);
    assert_eq!(
        attach_other.step(b"\r", 0, t0),
        Step {
            consumed: 1,
            intents: vec![Intent::Switch {
                to: "s2".to_owned(),
            }],
        }
    );
    let _ = attach_other.attached(
        "s2".to_owned(),
        Attached {
            in_team: true,
            granted_top: 2,
        },
    );
    let _ = attach_other.on_status(team_status("t", &["s1", "s2"]), t0);
    assert_eq!(
        attach_other.step(b"\x1b[<0;1;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );

    // 11. Grid press and claimed press require button == 0.
    let mut btn_guard = test_model(true, 2, false, None, t0);
    let _ = btn_guard.on_status(team_status("t", &["s1", "s2"]), t0);
    let _ = btn_guard.on_output(b"\x1b[?1000h".to_vec(), t0);
    assert_eq!(
        btn_guard.step(b"\x1b[<64;5;3M", 0, t0),
        Step {
            consumed: 10,
            intents: vec![Intent::Forward(b"\x1b[<64;5;1M".to_vec())],
        }
    );
    assert_eq!(
        btn_guard.step(b"\x1b[<0;5;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    assert_eq!(
        btn_guard.step(b"\x1b[<1;1;1M", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
    assert_eq!(
        btn_guard.step(b"\x1b[<0;1;1m", 0, t0),
        Step {
            consumed: 9,
            intents: vec![],
        }
    );
}
