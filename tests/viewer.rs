use a2amx::viewer::{Viewer, marker, overlay, prefix_name};
use std::time::{Duration, Instant};

#[test]
fn idle_expiry_counts_only_own_input_and_fires_once() {
    let start = Instant::now();
    let mut viewer = Viewer::new(start, false);
    assert!(!viewer.idle_expired(start + Duration::from_secs(4), Some(Duration::from_secs(5))));
    viewer.own_input(start + Duration::from_secs(4));
    assert!(!viewer.idle_expired(start + Duration::from_secs(8), Some(Duration::from_secs(5))));
    assert!(viewer.idle_expired(start + Duration::from_secs(9), Some(Duration::from_secs(5))));
    assert!(!viewer.idle_expired(
        start + Duration::from_secs(10),
        Some(Duration::from_secs(5))
    ));

    let mut viewer = Viewer::new(start, false);
    assert!(!viewer.idle_expired(start + Duration::from_secs(60), None));
    assert!(!viewer.is_viewer());
}

#[test]
fn toggle_round_trips_and_viewer_does_not_reset_idle_clock() {
    let start = Instant::now();
    let mut viewer = Viewer::new(start, false);
    assert!(viewer.toggle(start + Duration::from_secs(1)));
    viewer.own_input(start + Duration::from_secs(2));
    assert!(viewer.is_viewer());
    assert!(!viewer.toggle(start + Duration::from_secs(3)));
    assert!(!viewer.idle_expired(start + Duration::from_secs(7), Some(Duration::from_secs(5))));
    assert!(viewer.idle_expired(start + Duration::from_secs(8), Some(Duration::from_secs(5))));
}

#[test]
fn blocked_overlay_extends_and_expires_once() {
    let start = Instant::now();
    let mut viewer = Viewer::new(start, true);
    assert!(!viewer.overlay_active(start));
    viewer.blocked(start + Duration::from_secs(1));
    assert!(viewer.overlay_active(start + Duration::from_secs(2)));
    viewer.blocked(start);
    assert!(viewer.overlay_active(start + Duration::from_secs(2)));
    assert!(!viewer.take_overlay_end(start + Duration::from_secs(2)));
    assert!(viewer.take_overlay_end(start + Duration::from_secs(3)));
    assert!(!viewer.take_overlay_end(start + Duration::from_secs(4)));
    assert!(!viewer.overlay_active(start + Duration::from_secs(4)));
}

#[test]
fn prefix_names_cover_worked_control_forms() {
    assert_eq!(prefix_name(0), "Ctrl-Space");
    assert_eq!(prefix_name(1), "Ctrl-A");
    assert_eq!(prefix_name(2), "Ctrl-B");
    assert_eq!(prefix_name(26), "Ctrl-Z");
    assert_eq!(prefix_name(0x1b), "Ctrl-[");
    assert_eq!(prefix_name(0x1c), "Ctrl-\\");
    assert_eq!(prefix_name(0x1d), "Ctrl-]");
    assert_eq!(prefix_name(0x1e), "Ctrl-^");
    assert_eq!(prefix_name(0x1f), "Ctrl-_");
    assert_eq!(prefix_name(0x7f), "0x7f");
}

#[test]
fn overlay_matches_the_locked_worked_literal() {
    let expected = "\x1b7\x1b[10;20H\x1b[0m┌───────────────────────────────────────┐\x1b[11;20H│ VIEW MODE                             │\x1b[12;20H│ typing is off, press Ctrl-B v to type │\x1b[13;20H└───────────────────────────────────────┘\x1b8";
    assert_eq!(overlay(80, 22, 0, 0x02), expected.as_bytes());
    assert_eq!(
        overlay(80, 22, 0, 0),
        "\x1b7\x1b[10;18H\x1b[0m┌───────────────────────────────────────────┐\x1b[11;18H│ VIEW MODE                                 │\x1b[12;18H│ typing is off, press Ctrl-Space v to type │\x1b[13;18H└───────────────────────────────────────────┘\x1b8"
            .as_bytes()
    );
    assert_eq!(
        overlay(80, 22, 0, 0x1d),
        "\x1b7\x1b[10;20H\x1b[0m┌───────────────────────────────────────┐\x1b[11;20H│ VIEW MODE                             │\x1b[12;20H│ typing is off, press Ctrl-] v to type │\x1b[13;20H└───────────────────────────────────────┘\x1b8"
            .as_bytes()
    );
    assert!(overlay(40, 22, 0, 0x02).is_empty());
    assert!(overlay(80, 3, 0, 0x02).is_empty());
}

#[test]
fn marker_matches_the_locked_worked_literal_without_blink() {
    let expected = "\x1b7\x1b[23;74H\x1b[1;30;43m VIEW \x1b[0m\x1b8";
    let bytes = marker(80, 24);
    assert_eq!(bytes, expected.as_bytes());
    assert!(bytes.windows(4).all(|window| window != b"\x1b[5m"));
    assert!(bytes.windows(4).all(|window| window != b"\x1b[6m"));
    assert!(marker(11, 24).is_empty());
    assert!(marker(80, 3).is_empty());
}
