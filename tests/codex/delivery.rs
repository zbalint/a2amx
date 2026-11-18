use super::*;

const ENVELOPE_TEXT: &str = "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>";

fn waiting_on_approval() -> Value {
    json!({"type": "active", "activeFlags": ["waitingOnApproval"]})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_reaches_the_codex_thread_and_the_thread_item_becomes_the_receipt() {
    let case = Case::start_with(|fake| {
        fake.set_threads(&[("thread-1", idle())]);
        fake.auto_enter(true);
    })
    .await;
    let id = case.send().await;
    let info = case.wait_state(&id, "submitted").await;
    assert_eq!(
        common::eventually(|| async {
            let info = case.status(&id).await;
            (info.evidence.as_deref() == Some("native_receipt")).then_some(info.evidence)
        })
        .await,
        Some("native_receipt".to_owned())
    );
    assert_eq!(info.state, "submitted");
    assert_eq!(
        case.fake.turn_starts(),
        [TurnStart {
            thread: "thread-1".into(),
            client_id: "m_1".into(),
            text: ENVELOPE_TEXT.into(),
        }]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_loaded_thread_the_message_waits() {
    let case = Case::start().await;
    let id = case.send().await;
    let info = case.wait_reason(&id, "no_thread").await;
    assert_eq!(info.state, "pending");
    assert!(case.fake.turn_starts().is_empty());
    tokio::time::sleep(std::time::Duration::from_millis(10_200)).await;
    let held = common::eventually(|| async {
        let summary = case.summary().await;
        (summary.hold_reason.as_deref() == Some("needs_input") && summary.held).then_some(summary)
    })
    .await;
    assert_eq!(held.hold_reason.as_deref(), Some("needs_input"));
    case.fake.set_threads(&[("thread-1", idle())]);
    case.fake.auto_enter(true);
    common::eventually(|| async {
        let summary = case.summary().await;
        (summary.hold_reason.is_none() && !summary.held).then_some(())
    })
    .await;
    case.wait_state(&id, "submitted").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_approval_dialog_holds_delivery_until_it_closes() {
    let case = Case::start_with(|fake| {
        fake.set_threads(&[("thread-1", waiting_on_approval())]);
        fake.auto_enter(true);
    })
    .await;
    let id = case.send().await;
    let info = case.wait_reason(&id, "waiting_on_approval").await;
    assert_eq!(info.state, "pending");
    assert!(case.fake.turn_starts().is_empty());
    case.fake.set_status("thread-1", idle());
    case.wait_state(&id, "submitted").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_most_recently_updated_thread_receives_the_message() {
    let case = Case::start_with(|fake| {
        fake.set_threads(&[("old-thread", idle()), ("new-thread", idle())]);
        fake.auto_enter(true);
    })
    .await;
    let id = case.send().await;
    case.wait_state(&id, "submitted").await;
    assert_eq!(case.fake.turn_starts()[0].thread, "new-thread");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_message_waits_until_the_first_shows_up_in_the_thread() {
    let case = Case::start_with(|fake| fake.set_threads(&[("thread-1", idle())])).await;
    let first = case.send().await;
    case.wait_state(&first, "submitted").await;
    let second = case.send().await;
    let info = case.wait_reason(&second, "in_flight").await;
    assert_eq!(info.state, "pending");
    assert_eq!(case.fake.turn_starts().len(), 1);
    case.fake.enter("thread-1", "m_1", ENVELOPE_TEXT);
    case.wait_state(&second, "submitted").await;
    let starts = case.fake.turn_starts();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[1].client_id, "m_2");
    assert_eq!(
        case.status(&first).await.evidence.as_deref(),
        Some("native_receipt")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_thread_item_with_different_text_is_no_receipt() {
    let case = Case::start_with(|fake| fake.set_threads(&[("thread-1", idle())])).await;
    let first = case.send().await;
    case.wait_state(&first, "submitted").await;
    let second = case.send().await;
    case.fake.enter("thread-1", "m_1", "tampered text");
    case.wait_state(&second, "submitted").await;
    assert_eq!(
        case.status(&first).await.evidence.as_deref(),
        Some("write_complete")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_turn_makes_the_message_undeliverable() {
    let case = Case::start_with(|fake| {
        fake.set_threads(&[("thread-1", idle())]);
        fake.reject_turns("thread is shutting down");
    })
    .await;
    let id = case.send().await;
    let info = case.wait_state(&id, "undeliverable").await;
    assert_eq!(info.detail.as_deref(), Some("thread is shutting down"));
    assert_eq!(case.fake.turn_starts().len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dead_app_server_makes_the_message_wait() {
    let case = Case::start_with(|fake| {
        fake.set_threads(&[("thread-1", idle())]);
        fake.auto_enter(true);
    })
    .await;
    case.fake.stop();
    let id = case.send().await;
    let info = case.wait_reason(&id, "app_server_down").await;
    assert_eq!(info.state, "pending");
}
