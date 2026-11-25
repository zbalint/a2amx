#[path = "common/status_layout.rs"]
mod status_layout_helpers;
use a2amx::harness::Harness;
use a2amx::status;
use a2amx::wire::{Activity, QuotaInfo, RemoteLease, RemoteStatus};
use status_layout_helpers::{info, team_member, visible};

#[test]
fn adjacent_drop_boundaries_keep_team_harness_role_and_quota() {
    let mut status = info("a");
    status.team = Some(a2amx::messaging::TeamScope {
        name: "x".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    });
    status.harness = Some(Harness::Claude);
    assert_eq!(
        visible(&status::render(
            &status::StatusBar::new("a", Some(&status), 2),
            13,
            24
        )),
        " a │ claude "
    );

    status.role = Some("architect".into());
    assert_eq!(
        visible(&status::render(
            &status::StatusBar::new("a", Some(&status), 2),
            21,
            24
        )),
        " a │ role architect "
    );

    status.team = None;
    status.harness = None;
    status.quota = Some(QuotaInfo {
        five_hour_left: Some(15),
        ..Default::default()
    });
    assert_eq!(
        visible(&status::render(
            &status::StatusBar::new("a", Some(&status), 2),
            13,
            24
        )),
        " a │ 5h 15% "
    );
}

#[test]
fn adjacent_drop_boundaries_keep_activity_pending_hint_remote_and_confined() {
    let mut status = info("a");
    status.quota = Some(QuotaInfo {
        five_hour_left: Some(15),
        ..Default::default()
    });
    status.activity = Some(Activity::Working);
    assert_eq!(
        visible(&status::render(
            &status::StatusBar::new("a", Some(&status), 2),
            16,
            24
        )),
        " a │ ● working "
    );

    status.quota = None;
    status.pending = 2;
    assert_eq!(
        visible(&status::render(
            &status::StatusBar::new("a", Some(&status), 2),
            15,
            24
        )),
        " a  2 pending "
    );

    status.hold = Some("human_draft".into());
    assert_eq!(
        visible(&status::render(
            &status::StatusBar::new("a", Some(&status), 2),
            29,
            24
        )),
        " a  HELD human_draft · ^B r "
    );

    status.remote = Some(RemoteStatus {
        lease: RemoteLease::Active,
        gateway: true,
    });
    assert_eq!(
        visible(&status::render(
            &status::StatusBar::new("a", Some(&status), 2),
            34,
            24
        )),
        " a ● remote on  HELD human_draft "
    );
    status.confined = true;
    assert_eq!(
        visible(&status::render(
            &status::StatusBar::new("a", Some(&status), 2),
            44,
            24
        )),
        format!(" a [confined]{} HELD human_draft ", " ".repeat(12))
    );
}

#[test]
fn hold_and_fixed_cells_preserve_width_boundaries() {
    let mut status = info("a@h");
    status.remote = Some(RemoteStatus {
        lease: RemoteLease::Active,
        gateway: true,
    });
    status.pending = 1;
    status.hold = Some("human_draft".into());
    status.confined = true;
    assert_eq!(
        visible(&status::render(
            &status::StatusBar::new("a@h", Some(&status), 2),
            65,
            24
        )),
        " a@h [confined] ● remote on  1 pending  HELD human_draft · ^B r "
    );
    assert_eq!(
        visible(&status::render(
            &status::StatusBar::new("a@h", Some(&status), 2),
            20,
            24
        )),
        "  HELD human_draft "
    );
}

#[test]
fn hint_prefix_boundaries_are_exact() {
    let mut status = info("a");
    status.hold = Some("human_draft".into());
    for (prefix, expected) in [
        (1, " a  HELD human_draft · ^A r "),
        (26, " a  HELD human_draft · ^Z r "),
    ] {
        assert_eq!(
            visible(&status::render(
                &status::StatusBar::new("a", Some(&status), prefix),
                29,
                24
            )),
            expected
        );
    }
    for prefix in [0, 27] {
        assert_eq!(
            visible(&status::render(
                &status::StatusBar::new("a", Some(&status), prefix),
                22,
                24
            )),
            " a  HELD human_draft "
        );
    }
}

#[test]
fn team_regions_clip_skip_and_padding_at_the_width_boundary() {
    let members = [
        team_member("a", "alpha", Some(Activity::Idle), false),
        team_member("b", "beta", Some(Activity::Working), false),
    ];
    let clipped = status::render_team_line("a", "dev", &members, 20);
    let expected_regions = vec![
        status::TeamRegion {
            first_col: 1,
            last_col: 12,
            target: status::TeamTarget::Picker,
        },
        status::TeamRegion {
            first_col: 13,
            last_col: 19,
            target: status::TeamTarget::Session("a".into()),
        },
    ];
    assert_eq!(clipped.regions, expected_regions);
    let line = status::render_team_line("a", "dev", &members, 20);
    let expected_bytes = [
        b"\x1b7\x1b[1;1H\x1b[0m\x1b[2K\x1b[38;5;252;48;5;236m team dev \x1b[38;5;240m\xe2\x94\x82\x1b[38;5;252m \x1b[38;5;78m\xe2\x97\x8f\x1b[38;5;252m \x1b[1malpha\x1b[22m".as_slice(),
        b"\x1b[0m\x1b8",
    ]
    .concat();
    assert_eq!(line.bytes, expected_bytes);
}

#[test]
fn narrow_team_prefix_saturates_room_without_panicking() {
    let members = [
        team_member("a", "alpha", Some(Activity::Idle), false),
        team_member("b", "beta", Some(Activity::Working), false),
    ];
    let line = status::render_team_line("a", "1234567890123456", &members, 20);
    let expected_bytes = [
        b"\x1b7\x1b[1;1H\x1b[0m\x1b[2K\x1b[38;5;252;48;5;236m team 1234567890123".as_slice(),
        b"\x1b[0m\x1b8",
    ]
    .concat();
    assert_eq!(line.bytes, expected_bytes);
    let expected_regions = vec![status::TeamRegion {
        first_col: 1,
        last_col: 19,
        target: status::TeamTarget::Picker,
    }];
    assert_eq!(line.regions, expected_regions);
}

fn team_bytes(content: &[u8], padding: usize) -> Vec<u8> {
    let mut bytes = b"\x1b7\x1b[1;1H\x1b[0m\x1b[2K\x1b[38;5;252;48;5;236m".to_vec();
    bytes.extend_from_slice(content);
    bytes.extend_from_slice(" ".repeat(padding).as_bytes());
    bytes.extend_from_slice(b"\x1b[0m\x1b8");
    bytes
}

#[test]
fn fit_members_drops_last_non_current_member() {
    let members = [
        team_member("a", "alpha", Some(Activity::Idle), false),
        team_member("b", "bravo", Some(Activity::Working), false),
        team_member("c", "charlie", Some(Activity::Busy), false),
        team_member("d", "delta", Some(Activity::Waiting), false),
    ];
    let line = status::render_team_line("b", "dev", &members, 46);
    let expected_bytes = team_bytes(
        b" team dev \x1b[38;5;240m\xe2\x94\x82\x1b[38;5;252m \x1b[38;5;78m\xe2\x97\x8f\x1b[38;5;252m alpha  \x1b[38;5;75m\xe2\x97\x8f\x1b[38;5;252m \x1b[1mbravo\x1b[22m  \x1b[38;5;209m\xe2\x97\x8f\x1b[38;5;252m charlie   +1 ",
        0,
    );
    assert_eq!(line.bytes, expected_bytes);
    let expected_regions = vec![
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
            last_col: 30,
            target: status::TeamTarget::Session("b".into()),
        },
        status::TeamRegion {
            first_col: 31,
            last_col: 41,
            target: status::TeamTarget::Session("c".into()),
        },
        status::TeamRegion {
            first_col: 42,
            last_col: 45,
            target: status::TeamTarget::Picker,
        },
    ];
    assert_eq!(line.regions, expected_regions);
}

#[test]
fn fit_members_suffix_boundary_is_exact() {
    let members = [
        team_member("a", "alpha", Some(Activity::Idle), false),
        team_member("b", "bravo", Some(Activity::Working), false),
        team_member("c", "charlie", Some(Activity::Busy), false),
        team_member("d", "delta", Some(Activity::Waiting), false),
    ];
    let exact = status::render_team_line("b", "dev", &members, 46);
    let exact_bytes = team_bytes(
        b" team dev \x1b[38;5;240m\xe2\x94\x82\x1b[38;5;252m \x1b[38;5;78m\xe2\x97\x8f\x1b[38;5;252m alpha  \x1b[38;5;75m\xe2\x97\x8f\x1b[38;5;252m \x1b[1mbravo\x1b[22m  \x1b[38;5;209m\xe2\x97\x8f\x1b[38;5;252m charlie   +1 ",
        0,
    );
    assert_eq!(exact.bytes, exact_bytes);
    let exact_regions = vec![
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
            last_col: 30,
            target: status::TeamTarget::Session("b".into()),
        },
        status::TeamRegion {
            first_col: 31,
            last_col: 41,
            target: status::TeamTarget::Session("c".into()),
        },
        status::TeamRegion {
            first_col: 42,
            last_col: 45,
            target: status::TeamTarget::Picker,
        },
    ];
    assert_eq!(exact.regions, exact_regions);

    let narrower = status::render_team_line("b", "dev", &members, 45);
    let narrower_bytes = team_bytes(
        b" team dev \x1b[38;5;240m\xe2\x94\x82\x1b[38;5;252m \x1b[38;5;78m\xe2\x97\x8f\x1b[38;5;252m alpha  \x1b[38;5;75m\xe2\x97\x8f\x1b[38;5;252m \x1b[1mbravo\x1b[22m   +2 ",
        10,
    );
    assert_eq!(narrower.bytes, narrower_bytes);
    let narrower_regions = vec![
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
            last_col: 30,
            target: status::TeamTarget::Session("b".into()),
        },
        status::TeamRegion {
            first_col: 31,
            last_col: 34,
            target: status::TeamTarget::Picker,
        },
    ];
    assert_eq!(narrower.regions, narrower_regions);
}

#[test]
fn overflow_suffix_has_picker_region() {
    let members = [
        team_member("a", "alpha", Some(Activity::Idle), false),
        team_member("b", "bravo", Some(Activity::Working), false),
        team_member("c", "charlie", Some(Activity::Busy), false),
        team_member("d", "delta", Some(Activity::Waiting), false),
    ];
    let line = status::render_team_line("b", "dev", &members, 46);
    let expected_bytes = team_bytes(
        b" team dev \x1b[38;5;240m\xe2\x94\x82\x1b[38;5;252m \x1b[38;5;78m\xe2\x97\x8f\x1b[38;5;252m alpha  \x1b[38;5;75m\xe2\x97\x8f\x1b[38;5;252m \x1b[1mbravo\x1b[22m  \x1b[38;5;209m\xe2\x97\x8f\x1b[38;5;252m charlie   +1 ",
        0,
    );
    assert_eq!(line.bytes, expected_bytes);
    let expected_regions = vec![
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
            last_col: 30,
            target: status::TeamTarget::Session("b".into()),
        },
        status::TeamRegion {
            first_col: 31,
            last_col: 41,
            target: status::TeamTarget::Session("c".into()),
        },
        status::TeamRegion {
            first_col: 42,
            last_col: 45,
            target: status::TeamTarget::Picker,
        },
    ];
    assert_eq!(line.regions, expected_regions);
}

#[test]
fn region_at_terminal_width_is_one_column() {
    let members = [team_member("a", "alpha", Some(Activity::Idle), false)];
    let line = status::render_team_line("a", "dev", &members, 14);
    let expected_bytes = team_bytes(
        b" team dev \x1b[38;5;240m\xe2\x94\x82\x1b[38;5;252m \x1b[38;5;78m\xe2\x97\x8f\x1b[38;5;252m",
        0,
    );
    assert_eq!(line.bytes, expected_bytes);
    let expected_regions = vec![
        status::TeamRegion {
            first_col: 1,
            last_col: 12,
            target: status::TeamTarget::Picker,
        },
        status::TeamRegion {
            first_col: 13,
            last_col: 13,
            target: status::TeamTarget::Session("a".into()),
        },
    ];
    assert_eq!(line.regions, expected_regions);
}

#[test]
fn team_and_member_control_characters_are_sanitized() {
    let members = [team_member("a", "na\nme", Some(Activity::Idle), false)];
    let line = status::render_team_line("a", "te\x01am", &members, 40);
    let expected_bytes = team_bytes(
        b" team te?am \x1b[38;5;240m\xe2\x94\x82\x1b[38;5;252m \x1b[38;5;78m\xe2\x97\x8f\x1b[38;5;252m \x1b[1mna?me\x1b[22m  ",
        16,
    );
    assert_eq!(line.bytes, expected_bytes);
    let expected_regions = vec![
        status::TeamRegion {
            first_col: 1,
            last_col: 14,
            target: status::TeamTarget::Picker,
        },
        status::TeamRegion {
            first_col: 15,
            last_col: 23,
            target: status::TeamTarget::Session("a".into()),
        },
    ];
    assert_eq!(line.regions, expected_regions);
}
