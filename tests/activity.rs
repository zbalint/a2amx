use std::time::{Duration, Instant};

use a2amx::activity::{Thresholds, Tracker};

#[test]
fn idle_baseline_waits_for_idle_threshold_and_stays_silent() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Thresholds {
        working_after: Some(Duration::from_secs(120)),
        idle_after: Some(Duration::from_secs(30)),
    });

    for seconds in 0..=40 {
        assert_eq!(
            tracker.observe(base + Duration::from_secs(seconds), false),
            None,
            "idle baseline at t={seconds}s",
        );
    }
}

#[test]
fn working_transition_repeats_until_announced() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Thresholds {
        working_after: Some(Duration::from_secs(120)),
        idle_after: Some(Duration::from_secs(30)),
    });
    assert_eq!(tracker.observe(base, false), None);
    assert_eq!(tracker.observe(base + Duration::from_secs(30), false), None);
    assert_eq!(tracker.observe(base + Duration::from_secs(40), true), None);
    assert_eq!(tracker.observe(base + Duration::from_secs(159), true), None);
    assert_eq!(
        tracker.observe(base + Duration::from_secs(160), true),
        Some(true)
    );
    assert_eq!(
        tracker.observe(base + Duration::from_secs(161), true),
        Some(true)
    );
    tracker.announced(true);
    assert_eq!(tracker.observe(base + Duration::from_secs(162), true), None);
}

#[test]
fn idle_transition_repeats_until_announced() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Thresholds {
        working_after: Some(Duration::from_secs(120)),
        idle_after: Some(Duration::from_secs(30)),
    });
    assert_eq!(tracker.observe(base, true), None);
    assert_eq!(tracker.observe(base + Duration::from_secs(120), true), None);
    tracker.announced(true);
    assert_eq!(
        tracker.observe(base + Duration::from_secs(200), false),
        None
    );
    assert_eq!(
        tracker.observe(base + Duration::from_secs(229), false),
        None
    );
    assert_eq!(
        tracker.observe(base + Duration::from_secs(230), false),
        Some(false)
    );
    tracker.announced(false);
    assert_eq!(
        tracker.observe(base + Duration::from_secs(231), false),
        None
    );
}

#[test]
fn short_working_flap_coalesces_back_to_announced_idle() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Thresholds {
        working_after: Some(Duration::from_secs(120)),
        idle_after: Some(Duration::from_secs(30)),
    });
    assert_eq!(tracker.observe(base, false), None);
    assert_eq!(tracker.observe(base + Duration::from_secs(30), false), None);
    tracker.announced(false);
    assert_eq!(tracker.observe(base + Duration::from_secs(300), true), None);
    assert_eq!(tracker.observe(base + Duration::from_secs(350), true), None);
    assert_eq!(
        tracker.observe(base + Duration::from_secs(351), false),
        None
    );
    assert_eq!(
        tracker.observe(base + Duration::from_secs(381), false),
        None
    );
}

#[test]
fn disabled_working_direction_advances_reference_silently() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Thresholds {
        working_after: None,
        idle_after: Some(Duration::from_secs(30)),
    });
    assert_eq!(tracker.observe(base, true), None);
    assert_eq!(
        tracker.observe(base + Duration::from_secs(100), false),
        None
    );
    assert_eq!(
        tracker.observe(base + Duration::from_secs(130), false),
        Some(false)
    );
    tracker.announced(false);
    assert_eq!(tracker.observe(base + Duration::from_secs(135), true), None);
    assert_eq!(
        tracker.observe(base + Duration::from_secs(165), false),
        None
    );
    assert_eq!(
        tracker.observe(base + Duration::from_secs(195), false),
        Some(false)
    );
}

#[test]
fn disabled_idle_direction_advances_reference_silently() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Thresholds {
        working_after: Some(Duration::from_secs(120)),
        idle_after: None,
    });
    assert_eq!(tracker.observe(base + Duration::from_secs(0), false), None);
    assert_eq!(tracker.observe(base + Duration::from_secs(50), true), None);
    assert_eq!(
        tracker.observe(base + Duration::from_secs(170), true),
        Some(true)
    );
    tracker.announced(true);
    assert_eq!(
        tracker.observe(base + Duration::from_secs(180), false),
        None
    );
    assert_eq!(tracker.observe(base + Duration::from_secs(230), true), None);
    assert_eq!(
        tracker.observe(base + Duration::from_secs(350), true),
        Some(true)
    );
}

#[test]
fn pending_idle_notice_survives_unsettled_working_observation() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Thresholds {
        working_after: Some(Duration::from_secs(120)),
        idle_after: Some(Duration::from_secs(30)),
    });
    assert_eq!(tracker.observe(base, true), None);
    assert_eq!(tracker.observe(base + Duration::from_secs(120), true), None);
    tracker.announced(true);
    assert_eq!(
        tracker.observe(base + Duration::from_secs(130), false),
        None
    );
    assert_eq!(
        tracker.observe(base + Duration::from_secs(160), false),
        Some(false)
    );
    assert_eq!(
        tracker.observe(base + Duration::from_secs(190), true),
        Some(false)
    );
}

#[test]
fn returning_to_reference_before_announcement_cancels_notice() {
    let base = Instant::now();
    let mut tracker = Tracker::new(Thresholds {
        working_after: Some(Duration::from_secs(120)),
        idle_after: Some(Duration::from_secs(30)),
    });
    assert_eq!(tracker.observe(base, true), None);
    assert_eq!(tracker.observe(base + Duration::from_secs(120), true), None);
    tracker.announced(true);
    assert_eq!(
        tracker.observe(base + Duration::from_secs(130), false),
        None
    );
    assert_eq!(
        tracker.observe(base + Duration::from_secs(160), false),
        Some(false)
    );
    assert_eq!(
        tracker.observe(base + Duration::from_secs(200), true),
        Some(false)
    );
    assert_eq!(tracker.observe(base + Duration::from_secs(320), true), None);
}
