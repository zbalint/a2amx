use super::*;

fn instant(seconds: u64) -> Instant {
    Instant::now() + Duration::from_secs(seconds)
}

#[test]
fn presence_starts_away_and_grants_grace_after_last_interactive_ends() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(60);
    assert!(!presence.present(start));

    presence.attach_started(start, false);
    assert!(presence.present(start));
    presence.attach_ended(start + Duration::from_secs(1), false);
    assert!(presence.present(start + Duration::from_secs(60)));
    assert!(!presence.present(start + Duration::from_secs(61)));
}

#[test]
fn zero_away_seconds_is_immediate() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(0);
    presence.attach_started(start, false);
    assert!(presence.present(start));
    presence.attach_ended(start, false);
    assert!(!presence.present(start));
}

#[test]
fn viewer_attachments_do_not_count_as_presence() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(60);
    presence.attach_started(start, true);
    assert!(!presence.present(start));
    presence.mode_changed(start + Duration::from_secs(1), false);
    assert!(presence.present(start + Duration::from_secs(1)));
    presence.mode_changed(start + Duration::from_secs(2), true);
    assert!(presence.present(start + Duration::from_secs(2)));
    assert!(!presence.present(start + Duration::from_secs(62)));
}

#[test]
fn presence_cause_distinguishes_attached_grace_and_away() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(30);
    assert_eq!(presence.cause(start), None);
    presence.attach_started(start, false);
    assert_eq!(
        presence.cause(start),
        Some(messaging::PresenceCause::Attached)
    );
    let ended = start + Duration::from_secs(1);
    presence.attach_ended(ended, false);
    assert_eq!(
        presence.cause(ended + Duration::from_secs(29)),
        Some(messaging::PresenceCause::Grace)
    );
    assert_eq!(presence.cause(ended + Duration::from_secs(30)), None);
}

#[test]
fn multiple_interactive_attachments_keep_presence_until_the_last_ends() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(10);
    presence.attach_started(start, false);
    presence.attach_started(start + Duration::from_secs(1), false);
    presence.attach_ended(start + Duration::from_secs(2), false);
    assert!(presence.present(start + Duration::from_secs(2)));
    presence.attach_ended(start + Duration::from_secs(3), false);
    assert!(presence.present(start + Duration::from_secs(12)));
    assert!(!presence.present(start + Duration::from_secs(13)));
}

#[test]
fn poll_announces_each_present_to_away_transition_once() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(3);
    assert!(!presence.poll(start));
    presence.attach_started(start, false);
    assert!(!presence.poll(start));
    presence.attach_ended(start + Duration::from_secs(1), false);
    assert!(!presence.poll(start + Duration::from_secs(2)));
    assert!(presence.poll(start + Duration::from_secs(5)));
    assert!(!presence.poll(start + Duration::from_secs(6)));
    presence.attach_started(start + Duration::from_secs(7), false);
    assert!(!presence.poll(start + Duration::from_secs(7)));
    presence.attach_ended(start + Duration::from_secs(8), false);
    assert!(!presence.poll(start + Duration::from_secs(9)));
    assert!(presence.poll(start + Duration::from_secs(12)));
}

#[test]
fn five_grace_cycles_have_one_away_poll_transition() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(20);
    presence.attach_started(start, false);
    assert!(!presence.poll(start));
    for cycle in 0..5 {
        let ended = start + Duration::from_secs(2 + cycle * 2);
        presence.attach_ended(ended, false);
        assert!(!presence.poll(ended + Duration::from_secs(1)));
        let restarted = ended + Duration::from_secs(2);
        presence.attach_started(restarted, false);
        assert!(!presence.poll(restarted));
    }
    let final_ended = start + Duration::from_secs(12);
    presence.attach_ended(final_ended, false);
    assert!(!presence.poll(final_ended + Duration::from_secs(19)));
    assert!(presence.poll(final_ended + Duration::from_secs(21)));
    assert!(!presence.poll(final_ended + Duration::from_secs(22)));
}
