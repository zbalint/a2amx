use super::*;

async fn wait_for_message_state(home: &Path, id: &str, state: &str) {
    let home = home.to_owned();
    let id = id.to_owned();
    let state = state.to_owned();
    eventually(move || {
        let home = home.clone();
        let id = id.clone();
        let state = state.clone();
        async move {
            let mut client = Client::connect(&home).await.ok()?;
            let Response::Status { message } =
                client.request(Request::MessageStatus { id }).await.ok()?
            else {
                return None;
            };
            (message.state == state).then_some(())
        }
    })
    .await;
}

async fn wait_for_held(home: &Path, session: &str, held: bool) {
    let home = home.to_owned();
    let session = session.to_owned();
    eventually(move || {
        let home = home.clone();
        let session = session.clone();
        async move {
            let mut client = Client::connect(&home).await.ok()?;
            let Response::Sessions { sessions } = client.request(Request::List).await.ok()? else {
                return None;
            };
            sessions
                .into_iter()
                .find(|summary| summary.id == session)
                .filter(|summary| summary.held == held)
                .map(|_| ())
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_session_appears_in_the_list_table() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let created = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-plan",
            "--",
            "sh",
            "-c",
            "sleep 30",
        ],
        &[],
    )?;
    assert!(created.status.success());
    assert_eq!(created.stdout, b"s1\n");

    let output = run_binary(dir.path(), &["list"], &[])?;
    assert!(output.status.success());
    let listing = String::from_utf8(output.stdout)?;
    assert!(listing.starts_with(
        "ID  NAME        HARNESS  CONFINE  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE\n",
    ));
    let cells = common::list_cells(&listing, "s1");
    assert_eq!(cells.len(), 13);
    assert_eq!(
        &cells[..5],
        ["s1", "agent-plan", "generic", "off", "running"]
    );
    assert_eq!(cells[6], "working");
    for index in [5, 7] {
        let bytes = cells[index].as_bytes();
        assert!(bytes.len() >= 2);
        assert!(
            bytes[..bytes.len() - 1]
                .iter()
                .all(|byte| byte.is_ascii_digit())
        );
        assert!(matches!(bytes.last(), Some(b's' | b'm' | b'h')));
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_attached_cell_shows_viewer_and_interactive_modes() -> anyhow::Result<()> {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await?;
    let id = match admin
        .request(Request::NewSession {
            argv: vec!["cat".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: Vec::new(),
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            activity: None,
            heartbeat: None,
            name: None,
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            team: None,
            role: None,
            remote: None,
        })
        .await?
    {
        a2amx::wire::Response::Created { session } => session,
        response => anyhow::bail!("session creation failed: {response:?}"),
    };

    let viewer = Client::connect(dir.path())
        .await?
        .attach_viewer(&id, false, 80, 20, 0)
        .await?;
    let listed = run_binary(dir.path(), &["list", "--details"], &[])?;
    anyhow::ensure!(listed.status.success(), "list failed: {:?}", listed);
    let listing = String::from_utf8(listed.stdout)?;
    let header = listing
        .lines()
        .find(|line| line.split_whitespace().next() == Some("ID"))
        .context("list header missing")?;
    let attached = header
        .split_whitespace()
        .position(|column| column == "ATTACHED")
        .context("ATTACHED header missing")?;
    assert_eq!(common::list_cells(&listing, &id)[attached], "viewer");
    drop(viewer);

    let interactive = Client::connect(dir.path())
        .await?
        .attach(&id, false, 80, 20)
        .await?;
    let listed = run_binary(dir.path(), &["list", "--details"], &[])?;
    anyhow::ensure!(listed.status.success(), "list failed: {:?}", listed);
    let listing = String::from_utf8(listed.stdout)?;
    let header = listing
        .lines()
        .find(|line| line.split_whitespace().next() == Some("ID"))
        .context("list header missing")?;
    let attached = header
        .split_whitespace()
        .position(|column| column == "ATTACHED")
        .context("ATTACHED header missing")?;
    assert_eq!(common::list_cells(&listing, &id)[attached], "yes");
    drop(interactive);
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attached_input_sets_hold_and_prefix_release_clears_it() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let created = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--deliver",
            "auto",
            "--",
            "sh",
            "-c",
            "stty raw -echo; cat",
        ],
        &[],
    )?;
    assert!(created.status.success());
    assert_eq!(created.stdout, b"s1\n");

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.send(b"x")?;
    wait_for_held(dir.path(), "s1", true).await;
    let listed = run_binary(dir.path(), &["list", "--details"], &[])?;
    assert_eq!(listed.status.code(), Some(0));
    let listing = String::from_utf8(listed.stdout)?;
    let cells = common::list_cells(&listing, "s1");
    assert_eq!(cells[0], "s1");
    assert_ne!(cells[1], "-");
    a2amx::messaging::validate_name(cells[1])?;
    assert_eq!(&cells[2..5], ["generic", "off", "running"]);
    assert_eq!(cells[6], "busy");
    assert_eq!(cells[8], "yes");
    assert_eq!(cells[9], "0");
    assert_eq!(cells[10], "human_draft");
    assert_eq!(cells[11], "-");
    assert_eq!(cells[12], "80x22");
    for index in [5, 7] {
        let bytes = cells[index].as_bytes();
        assert!(bytes.len() >= 2);
        assert!(
            bytes[..bytes.len() - 1]
                .iter()
                .all(|byte| byte.is_ascii_digit())
        );
        assert!(matches!(bytes.last(), Some(b's' | b'm' | b'h')));
    }
    assert!(listing.contains(&format!(
        "80x22  {}  sh -c stty raw -echo; cat\n",
        std::env::current_dir()?.display(),
    )));

    attached.send(&[2, b'r'])?;
    wait_for_held(dir.path(), "s1", false).await;
    let listed = run_binary(dir.path(), &["list"], &[])?;
    assert!(listed.status.success());
    let listing = String::from_utf8(listed.stdout)?;
    let cells = common::list_cells(&listing, "s1");
    assert_eq!(cells[0], "s1");
    assert_ne!(cells[1], "-");
    a2amx::messaging::validate_name(cells[1])?;
    assert_eq!(&cells[2..5], ["generic", "off", "running"]);
    assert_eq!(cells[6], "working");
    assert_eq!(cells[8], "yes");
    assert_eq!(cells[9], "0");
    assert_eq!(cells[10], "-");

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_cli_prints_submitted_and_undeliverable_rows() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await?;
    let (_, sender_token, sender_addr) = new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let recipient_output = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-review",
            "--harness",
            "generic",
            "--deliver",
            "auto",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; stty raw -echo; cat >/dev/null",
        ],
    )?;
    assert_eq!(recipient_output.1, 0);
    let mut sender = Client::connect_addr(sender_addr, &sender_token).await?;
    let accepted = sender
        .request(Request::SendMessage {
            to: "agent-review@host-a".into(),
            subject: "Parser issue".into(),
            message: "I found the regression.".into(),
        })
        .await?;
    assert_eq!(
        accepted,
        Response::Accepted {
            id: "m_1".into(),
            recipient_hold: None,
            recipient_quota: None
        }
    );
    wait_for_message_state(dir.path(), "m_1", "submitted").await;
    let (hold_session, _, _) = new_agent(
        &mut admin,
        dir.path(),
        Some("agent-hold"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    assert_eq!(hold_session, "s3");

    let accepted = sender
        .request(Request::SendMessage {
            to: "agent-hold@host-a".into(),
            subject: "Second".into(),
            message: "Another issue.".into(),
        })
        .await?;
    assert_eq!(
        accepted,
        Response::Accepted {
            id: "m_2".into(),
            recipient_hold: Some("deliver_hold".into()),
            recipient_quota: None
        }
    );
    let accepted = sender
        .request(Request::SendMessage {
            to: "agent-hold@host-a".into(),
            subject: "Trailing ".into(),
            message: "Subjects print as stored.".into(),
        })
        .await?;
    assert_eq!(
        accepted,
        Response::Accepted {
            id: "m_3".into(),
            recipient_hold: Some("deliver_hold".into()),
            recipient_quota: None
        }
    );
    assert_eq!(
        admin
            .request(Request::Kill {
                session: hold_session,
                now: true,
            })
            .await?,
        Response::Ok
    );
    wait_for_message_state(dir.path(), "m_2", "undeliverable").await;
    wait_for_message_state(dir.path(), "m_3", "undeliverable").await;

    let output = run_binary(dir.path(), &["messages"], &[])?;
    assert!(output.status.success());
    let listing = String::from_utf8(output.stdout)?;
    assert!(listing.starts_with(
        "ID   FROM               TO                   STATE          DETAIL            SUBJECT\n"
    ));
    assert!(listing.contains("m_1  agent-plan@host-a  agent-review@host-a  submitted      -                 Parser issue\n"));
    assert!(listing.contains(
        "m_2  agent-plan@host-a  agent-hold@host-a    undeliverable  recipient_exited  Second\n"
    ));
    assert!(listing.contains(
        "m_3  agent-plan@host-a  agent-hold@host-a    undeliverable  recipient_exited  Trailing \n"
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_is_silent_on_success_and_reports_errors() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await?;
    let (_, sender_token, sender_addr) = new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_, _, _) = new_agent(
        &mut admin,
        dir.path(),
        Some("agent-review"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(sender_addr, &sender_token).await?;
    assert_eq!(
        sender
            .request(Request::SendMessage {
                to: "agent-review@host-a".into(),
                subject: "Pending".into(),
                message: "Cancel me.".into(),
            })
            .await?,
        Response::Accepted {
            id: "m_1".into(),
            recipient_hold: Some("deliver_hold".into()),
            recipient_quota: None
        }
    );

    let cancelled = run_binary(dir.path(), &["cancel", "m_1"], &[])?;
    assert_eq!(cancelled.status.code(), Some(0));
    assert!(cancelled.stdout.is_empty());
    assert!(cancelled.stderr.is_empty());

    let missing = run_binary(dir.path(), &["cancel", "m_99"], &[])?;
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    assert_eq!(
        String::from_utf8(missing.stderr)?,
        "a2amx: unknown message m_99\n"
    );
    Ok(())
}
