use a2amx::status;

const PRE: &str = "\x1b7\x1b[24;1H\x1b[0m\x1b[7m";
const POST: &str = "\x1b[0m\x1b8";

#[test]
fn fallback_session_is_padded_and_cursor_is_restored() {
    assert_eq!(
        status::render("s1", None, 40, 24),
        format!("{PRE} s1 {}{POST}", "                                   ").as_bytes()
    );
}

#[test]
fn pending_count_is_right_aligned_with_the_address() {
    let info = a2amx::wire::StatusInfo {
        address: "agent-plan@host-a".into(),
        pending: 2,
        hold: None,
    };
    assert_eq!(
        status::render("s1", Some(&info), 40, 24),
        format!("{PRE} agent-plan@host-a {} 2 pending {POST}", "         ").as_bytes()
    );
}

#[test]
fn hold_alert_keeps_priority_over_pending_and_address() {
    let info = a2amx::wire::StatusInfo {
        address: "agent-plan@host-a".into(),
        pending: 2,
        hold: Some("human_draft".into()),
    };
    assert_eq!(
        status::render("s1", Some(&info), 40, 24),
        format!("{PRE} agent-plan@host-a   \x1b[0m\x1b[1;37;41m HELD human_draft {POST}")
            .as_bytes()
    );
    assert_eq!(
        status::render("s1", Some(&info), 20, 24),
        format!("{PRE} \x1b[0m\x1b[1;37;41m HELD human_draft {POST}").as_bytes()
    );
    assert_eq!(
        status::render("s1", Some(&info), 8, 24),
        format!("{PRE}\x1b[0m\x1b[1;37;41m HELD h{POST}").as_bytes()
    );
}

#[test]
fn daemon_text_controls_are_replaced_before_layout() {
    let info = a2amx::wire::StatusInfo {
        address: "a\x1bb".into(),
        pending: 0,
        hold: None,
    };
    assert_eq!(
        status::render("s1", Some(&info), 40, 24),
        format!("{PRE} a?b {}{POST}", "                                  ").as_bytes()
    );
    assert_eq!(
        status::render("a\nb", None, 6, 24),
        format!("{PRE} a?b {POST}").as_bytes()
    );
    let info = a2amx::wire::StatusInfo {
        address: "a".into(),
        pending: 0,
        hold: Some("x\n".into()),
    };
    assert_eq!(
        status::render("s1", Some(&info), 10, 24),
        format!("{PRE}\x1b[0m\x1b[1;37;41m HELD x? {POST}").as_bytes()
    );
}

#[test]
fn tiny_terminals_do_not_draw_a_bar() {
    assert_eq!(status::render("s1", None, 40, 2), Vec::<u8>::new());
    assert_eq!(status::render("s1", None, 1, 24), Vec::<u8>::new());
    assert_eq!(status::render("s1", None, 0, 0), Vec::<u8>::new());
}
