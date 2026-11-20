use a2amx::harness::Harness;
use a2amx::lookup::resolve_reference;
use a2amx::wire::SessionSummary;

fn session_with_name(id: &str, name: Option<&str>) -> SessionSummary {
    SessionSummary {
        id: id.to_owned(),
        argv: vec!["sh".to_owned()],
        cols: 80,
        rows: 24,
        exit_code: None,
        attached: false,
        viewer: false,
        name: name.map(str::to_owned),
        address: String::new(),
        pending: 0,
        held: false,
        hold_reason: None,
        quota: None,
        harness: Harness::Generic,
        confine: None,
        team: None,
        cwd: None,
        activity: None,
        uptime_secs: None,
        activity_secs: None,
        starting: false,
    }
}

#[test]
fn resolve_reference_maps_a_name_to_its_id_and_leaves_other_references() {
    let sessions = [
        session_with_name("s1", Some("alpha")),
        session_with_name("s2", None),
    ];
    assert_eq!(resolve_reference(&sessions, "alpha"), "s1");
    assert_eq!(resolve_reference(&sessions, "s1"), "s1");
    assert_eq!(resolve_reference(&sessions, "unknown"), "unknown");
}
