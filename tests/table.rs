use a2amx::harness::Harness;
use a2amx::messaging::TeamScope;
use a2amx::table::{
    format_explain, format_messages_table, format_session_table, session_confinement_mode,
};
use a2amx::wire::{
    Activity, ConfineInfo, ExplainInfo, HoldInfo, MessageInfo, ResetVerdict, SessionSummary,
};

fn base_message(id: &str, from: &str, to: &str, state: &str, subject: &str) -> MessageInfo {
    MessageInfo {
        id: id.to_owned(),
        from: from.to_owned(),
        to: to.to_owned(),
        subject: subject.to_owned(),
        state: state.to_owned(),
        detail: None,
        hold_reason: None,
        evidence: None,
        hold_explanation: None,
        accepted_at: None,
        updated_at: None,
    }
}

fn base_session(id: &str, name: Option<&str>) -> SessionSummary {
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
        harness: Harness::Claude,
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
fn messages_table_aligns_columns_to_the_widest_cell() {
    let messages = [base_message("m_1", "a@h", "b@h", "delivered", "hi")];
    assert_eq!(
        format_messages_table(&messages),
        "ID   FROM  TO   STATE      DETAIL  SUBJECT\nm_1  a@h   b@h  delivered  -       hi\n"
    );
}

#[test]
fn message_detail_prefers_detail_then_hold_reason_then_receipt_evidence() {
    let mut m1 = base_message("m1", "a@h", "b@h", "held", "s");
    m1.detail = Some("d".to_owned());
    m1.hold_reason = Some("h".to_owned());

    let mut m2 = base_message("m2", "a@h", "b@h", "held", "s");
    m2.hold_reason = Some("h".to_owned());
    m2.evidence = Some("native_receipt".to_owned());

    let mut m3 = base_message("m3", "a@h", "b@h", "held", "s");
    m3.evidence = Some("submission_observed".to_owned());

    let m4 = base_message("m4", "a@h", "b@h", "held", "s");

    assert_eq!(
        format_messages_table(&[m1, m2, m3, m4]),
        concat!(
            "ID  FROM  TO   STATE  DETAIL               SUBJECT\n",
            "m1  a@h   b@h  held   d                    s\n",
            "m2  a@h   b@h  held   h                    s\n",
            "m3  a@h   b@h  held   submission_observed  s\n",
            "m4  a@h   b@h  held   -                    s\n",
        )
    );
}

#[test]
fn message_detail_ignores_other_evidence() {
    let mut message = base_message("m_1", "a@h", "b@h", "held", "hi");
    message.evidence = Some("other".to_owned());
    assert_eq!(
        format_messages_table(&[message]),
        "ID   FROM  TO   STATE  DETAIL  SUBJECT\nm_1  a@h   b@h  held   -       hi\n"
    );
}

#[test]
fn message_detail_shows_native_receipt_evidence_alone() {
    let mut message = base_message("m_1", "a@h", "b@h", "held", "hi");
    message.evidence = Some("native_receipt".to_owned());
    assert_eq!(
        format_messages_table(&[message]),
        "ID   FROM  TO   STATE  DETAIL          SUBJECT\nm_1  a@h   b@h  held   native_receipt  hi\n"
    );
}

#[test]
fn a_long_value_widens_its_column_and_shorter_rows_are_padded() {
    let messages = [
        base_message("m_1", "very-long-sender@host-a", "b@h", "pending", "one"),
        base_message("m_2", "a@h", "b@h", "delivered", "two"),
    ];
    assert_eq!(
        format_messages_table(&messages),
        concat!(
            "ID   FROM                     TO   STATE      DETAIL  SUBJECT\n",
            "m_1  very-long-sender@host-a  b@h  pending    -       one\n",
            "m_2  a@h                      b@h  delivered  -       two\n",
        )
    );
}

#[test]
fn session_table_has_a_team_column_only_when_a_session_has_a_team() {
    let s1 = base_session("s1", Some("a"));
    assert_eq!(
        format_session_table(std::slice::from_ref(&s1), false),
        concat!(
            "ID  NAME  HARNESS  CONFINE  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE\n",
            "s1  a     claude   off      running  -       -         -         no        0        -     -      80x24\n",
        )
    );

    let mut s2 = base_session("s2", Some("b"));
    s2.team = Some(TeamScope {
        name: "demo".to_owned(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    });
    assert_eq!(
        format_session_table(&[s1, s2], false),
        concat!(
            "ID  NAME  TEAM            HARNESS  CONFINE  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE\n",
            "s1  a     -               claude   off      running  -       -         -         no        0        -     -      80x24\n",
            "s2  b     demo (private)  claude   off      running  -       -         -         no        0        -     -      80x24\n",
        )
    );
}

#[test]
fn session_table_details_add_cwd_and_command_columns() {
    let mut with_cwd = base_session("s1", Some("a"));
    with_cwd.cwd = Some("/w".to_owned());
    with_cwd.argv = vec!["cat".to_owned(), "-n".to_owned()];
    assert_eq!(
        format_session_table(&[with_cwd], true),
        "ID  NAME  HARNESS  CONFINE  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE   CWD  COMMAND\ns1  a     claude   off      running  -       -         -         no        0        -     -      80x24  /w   cat -n\n"
    );

    let mut without_cwd = base_session("s1", Some("a"));
    without_cwd.argv = vec!["cat".to_owned(), "-n".to_owned()];
    assert_eq!(
        format_session_table(&[without_cwd], true),
        "ID  NAME  HARNESS  CONFINE  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE   CWD  COMMAND\ns1  a     claude   off      running  -       -         -         no        0        -     -      80x24  -    cat -n\n"
    );

    let mut waiting = base_session("s2", Some("b"));
    waiting.harness = Harness::Agy;
    waiting.activity = Some(Activity::Waiting);
    waiting.uptime_secs = Some(25);
    waiting.activity_secs = Some(12);
    assert_eq!(
        format_session_table(&[waiting], false),
        "ID  NAME  HARNESS  CONFINE  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE\ns2  b     agy      off      running  25s     waiting   12s       no        0        -     -      80x24\n"
    );
}

#[test]
fn confinement_mode_label() {
    let mut session = base_session("s1", Some("a"));
    assert_eq!(session_confinement_mode(&session), "off");

    session.confine = Some(ConfineInfo {
        auto: true,
        grants: Vec::new(),
        write: Vec::new(),
        reasons: Vec::new(),
        hide: Vec::new(),
        unhide: Vec::new(),
        no_default_hides: false,
    });
    assert_eq!(session_confinement_mode(&session), "auto");

    session.confine.as_mut().unwrap().auto = false;
    assert_eq!(session_confinement_mode(&session), "manual");
}

#[test]
fn explain_exited_session_is_one_line() {
    let info = ExplainInfo {
        id: "s1".to_owned(),
        address: "agent-plan@host-a".to_owned(),
        harness: Harness::Generic,
        exit_code: Some(3),
        ready: None,
        starting: false,
        activity: None,
        activity_cause: None,
        needs_input: None,
        hold: None,
        pending: 0,
        reset: ResetVerdict {
            allowed: false,
            code: None,
            reason: None,
        },
    };
    assert_eq!(
        format_explain(&info),
        "s1 (agent-plan@host-a): generic, exited (code 3)\n"
    );
}

#[test]
fn explain_running_session_with_unknowns_and_open_gate() {
    let info = ExplainInfo {
        id: "s1".to_owned(),
        address: "agent-plan@host-a".to_owned(),
        harness: Harness::Generic,
        exit_code: None,
        ready: None,
        starting: false,
        activity: None,
        activity_cause: None,
        needs_input: None,
        hold: None,
        pending: 0,
        reset: ResetVerdict {
            allowed: true,
            code: None,
            reason: None,
        },
    };
    assert_eq!(
        format_explain(&info),
        concat!(
            "s1 (agent-plan@host-a): generic, running\n",
            "activity: unknown (unknown)\n",
            "ready: unknown\n",
            "needs input: no\n",
            "hold: none\n",
            "queued: 0\n",
            "reset: gate open\n",
        )
    );
}

#[test]
fn explain_running_session_with_hold_and_refused_reset() {
    let mut info = ExplainInfo {
        id: "s1".to_owned(),
        address: "agent-plan@host-a".to_owned(),
        harness: Harness::Generic,
        exit_code: None,
        ready: Some(true),
        starting: false,
        activity: Some(Activity::Working),
        activity_cause: Some("output".to_owned()),
        needs_input: Some("trust prompt".to_owned()),
        hold: Some(HoldInfo {
            reason: "recipient_busy".to_owned(),
            explanation: Some("composer has a draft".to_owned()),
        }),
        pending: 2,
        reset: ResetVerdict {
            allowed: false,
            code: Some("not_ready".to_owned()),
            reason: Some("screen not ready".to_owned()),
        },
    };
    assert_eq!(
        format_explain(&info),
        concat!(
            "s1 (agent-plan@host-a): generic, running\n",
            "activity: working (output)\n",
            "ready: yes\n",
            "needs input: trust prompt\n",
            "hold: recipient_busy - composer has a draft\n",
            "queued: 2\n",
            "reset: refused (not_ready): screen not ready\n",
        )
    );

    info.hold = Some(HoldInfo {
        reason: "recipient_busy".to_owned(),
        explanation: None,
    });
    info.reset = ResetVerdict {
        allowed: false,
        code: None,
        reason: None,
    };
    assert_eq!(
        format_explain(&info),
        concat!(
            "s1 (agent-plan@host-a): generic, running\n",
            "activity: working (output)\n",
            "ready: yes\n",
            "needs input: trust prompt\n",
            "hold: recipient_busy\n",
            "queued: 2\n",
            "reset: refused (unknown): unknown\n",
        )
    );
}

#[test]
fn explain_ready_states() {
    let ready_yes = ExplainInfo {
        id: "s1".to_owned(),
        address: "agent-plan@host-a".to_owned(),
        harness: Harness::Generic,
        exit_code: None,
        ready: Some(true),
        starting: false,
        activity: Some(Activity::Idle),
        activity_cause: Some("prompt".to_owned()),
        needs_input: None,
        hold: None,
        pending: 0,
        reset: ResetVerdict {
            allowed: true,
            code: None,
            reason: None,
        },
    };
    assert_eq!(
        format_explain(&ready_yes),
        concat!(
            "s1 (agent-plan@host-a): generic, running\n",
            "activity: idle (prompt)\n",
            "ready: yes\n",
            "needs input: no\n",
            "hold: none\n",
            "queued: 0\n",
            "reset: gate open\n",
        )
    );

    let ready_starting = ExplainInfo {
        id: "s1".to_owned(),
        address: "agent-plan@host-a".to_owned(),
        harness: Harness::Generic,
        exit_code: None,
        ready: Some(false),
        starting: true,
        activity: Some(Activity::Busy),
        activity_cause: Some("boot".to_owned()),
        needs_input: None,
        hold: None,
        pending: 0,
        reset: ResetVerdict {
            allowed: true,
            code: None,
            reason: None,
        },
    };
    assert_eq!(
        format_explain(&ready_starting),
        concat!(
            "s1 (agent-plan@host-a): generic, running\n",
            "activity: busy (boot)\n",
            "ready: no (starting)\n",
            "needs input: no\n",
            "hold: none\n",
            "queued: 0\n",
            "reset: gate open\n",
        )
    );

    let ready_no = ExplainInfo {
        id: "s1".to_owned(),
        address: "agent-plan@host-a".to_owned(),
        harness: Harness::Generic,
        exit_code: None,
        ready: Some(false),
        starting: false,
        activity: Some(Activity::Working),
        activity_cause: Some("output".to_owned()),
        needs_input: None,
        hold: None,
        pending: 0,
        reset: ResetVerdict {
            allowed: true,
            code: None,
            reason: None,
        },
    };
    assert_eq!(
        format_explain(&ready_no),
        concat!(
            "s1 (agent-plan@host-a): generic, running\n",
            "activity: working (output)\n",
            "ready: no\n",
            "needs input: no\n",
            "hold: none\n",
            "queued: 0\n",
            "reset: gate open\n",
        )
    );

    let waiting = ExplainInfo {
        id: "s2".to_owned(),
        address: "agy-dev@host-a".to_owned(),
        harness: Harness::Agy,
        exit_code: None,
        ready: Some(true),
        starting: false,
        activity: Some(Activity::Waiting),
        activity_cause: Some("background job running".to_owned()),
        needs_input: None,
        hold: None,
        pending: 0,
        reset: ResetVerdict {
            allowed: true,
            code: None,
            reason: None,
        },
    };
    assert_eq!(
        format_explain(&waiting),
        concat!(
            "s2 (agy-dev@host-a): agy, running\n",
            "activity: waiting (background job running)\n",
            "ready: yes\n",
            "needs input: no\n",
            "hold: none\n",
            "queued: 0\n",
            "reset: gate open\n",
        )
    );
}
