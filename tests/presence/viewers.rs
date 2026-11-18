use super::*;

async fn session_held(admin: &mut Client, session: &str) -> bool {
    match admin.request(Request::List).await.unwrap() {
        Response::Sessions { sessions } => sessions
            .into_iter()
            .find(|candidate| candidate.id == session)
            .map(|candidate| candidate.held)
            .unwrap(),
        response => panic!("unexpected list response: {response:?}"),
    }
}

#[tokio::test]
async fn viewer_attach_keeps_input_read_only_while_sizing_session() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(Limits::default()).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = match admin
        .request(new_session_request(None, &["cat"]))
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("session creation failed: {response:?}"),
    };
    let mut viewer = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_viewer(&id, false, 80, 20, 0)
        .await
        .unwrap();
    let sessions = match admin.request(Request::List).await.unwrap() {
        Response::Sessions { sessions } => sessions,
        response => panic!("list failed: {response:?}"),
    };
    let session = sessions
        .into_iter()
        .find(|session| session.id == id)
        .unwrap();
    assert_eq!((session.cols, session.rows), (80, 20));
    let viewer_agent = match admin.request(Request::ListAgents).await.unwrap() {
        Response::Agents { agents, .. } => {
            assert_eq!(agents.len(), 1);
            agents.into_iter().next().unwrap()
        }
        response => panic!("unexpected list_agents response: {response:?}"),
    };
    assert!(viewer_agent.attached);
    viewer
        .send(ClientFrame::Resize {
            cols: 100,
            rows: 30,
        })
        .await
        .unwrap();
    receive_data(&mut viewer).await;
    let sessions = match admin.request(Request::List).await.unwrap() {
        Response::Sessions { sessions } => sessions,
        response => panic!("list failed: {response:?}"),
    };
    let session = sessions
        .into_iter()
        .find(|session| session.id == id)
        .unwrap();
    assert_eq!((session.cols, session.rows), (100, 30));
    viewer
        .send(ClientFrame::Scroll(Scroll::LineUp))
        .await
        .unwrap();
    receive_data(&mut viewer).await;
    viewer.send(ClientFrame::Redraw).await.unwrap();
    receive_data(&mut viewer).await;
    viewer.send(ClientFrame::Release).await.unwrap();
    viewer
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let before = match admin
        .request(Request::Screen {
            session: id.clone(),
        })
        .await
        .unwrap()
    {
        Response::Screen { lines } => lines.join("\n"),
        response => panic!("screen failed: {response:?}"),
    };
    assert!(!before.contains('x'));
    viewer
        .send(ClientFrame::Mode { viewer: false })
        .await
        .unwrap();
    viewer
        .send(ClientFrame::Input(b"y".to_vec()))
        .await
        .unwrap();
    let mut found_y = false;
    for _ in 0..100 {
        let response = admin
            .request(Request::Screen {
                session: id.clone(),
            })
            .await
            .unwrap();
        let Response::Screen { lines } = response else {
            panic!("screen failed");
        };
        if lines.join("\n").contains('y') {
            found_y = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(found_y, "interactive mode input was not rendered");
    let sessions = match admin.request(Request::List).await.unwrap() {
        Response::Sessions { sessions } => sessions,
        response => panic!("list failed: {response:?}"),
    };
    let session = sessions
        .into_iter()
        .find(|session| session.id == id)
        .unwrap();
    assert_eq!((session.cols, session.rows), (100, 30));
    drop(viewer);
    daemon.shutdown().await.unwrap();
}
#[tokio::test]
async fn viewer_attach_is_marked_in_session_and_agent_lists() {
    let (dir, daemon) = start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = match admin
        .request(new_session_request(None, &["cat"]))
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("session creation failed: {response:?}"),
    };
    let mut viewer = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_viewer(&id, false, 80, 20, 0)
        .await
        .unwrap();
    receive_data(&mut viewer).await;

    async fn flags(admin: &mut Client, id: &str) -> ((bool, bool), (bool, bool)) {
        let session = match admin.request(Request::List).await.unwrap() {
            Response::Sessions { sessions } => sessions
                .into_iter()
                .find(|session| session.id == id)
                .unwrap(),
            response => panic!("list failed: {response:?}"),
        };
        let agent = match admin.request(Request::ListAgents).await.unwrap() {
            Response::Agents { mut agents, .. } => agents.pop().unwrap(),
            response => panic!("list_agents failed: {response:?}"),
        };
        let session = serde_json::to_value(session).unwrap();
        let agent = serde_json::to_value(agent).unwrap();
        (
            (
                session["attached"].as_bool().unwrap(),
                session["viewer"].as_bool().unwrap_or(false),
            ),
            (
                agent["attached"].as_bool().unwrap(),
                agent["viewer"].as_bool().unwrap_or(false),
            ),
        )
    }

    assert_eq!(flags(&mut admin, &id).await, ((true, true), (true, true)));
    viewer
        .send(ClientFrame::Mode { viewer: false })
        .await
        .unwrap();
    viewer.send(ClientFrame::Redraw).await.unwrap();
    receive_data(&mut viewer).await;
    let false_flags = flags(&mut admin, &id).await;
    assert_eq!(false_flags, ((true, false), (true, false)));

    viewer
        .send(ClientFrame::Mode { viewer: true })
        .await
        .unwrap();
    viewer.send(ClientFrame::Redraw).await.unwrap();
    receive_data(&mut viewer).await;
    assert_eq!(flags(&mut admin, &id).await, ((true, true), (true, true)));

    drop(viewer);
    let mut detached = false;
    for _ in 0..100 {
        if flags(&mut admin, &id).await == ((false, false), (false, false)) {
            detached = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(detached, "attachment slot did not clear");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn release_clears_interactive_hold_but_viewer_release_is_ignored() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(Limits::default()).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = match admin
        .request(new_session_request(None, &["cat"]))
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("session creation failed: {response:?}"),
    };
    let mut interactive = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 10)
        .await
        .unwrap();
    interactive
        .send(ClientFrame::Input(b"draft".to_vec()))
        .await
        .unwrap();
    let mut held = false;
    for _ in 0..100 {
        if session_held(&mut admin, &id).await {
            held = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(held, "interactive input creates a human-draft hold");
    interactive.send(ClientFrame::Release).await.unwrap();
    let mut released = false;
    for _ in 0..100 {
        if !session_held(&mut admin, &id).await {
            released = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(released, "interactive Release clears the hold");
    interactive
        .send(ClientFrame::Input(b"draft again".to_vec()))
        .await
        .unwrap();
    let mut held_again = false;
    for _ in 0..100 {
        if session_held(&mut admin, &id).await {
            held_again = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(held_again, "interactive input recreates the hold");
    let mut viewer = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_viewer(&id, true, 40, 10, 0)
        .await
        .unwrap();
    viewer.send(ClientFrame::Release).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(session_held(&mut admin, &id).await);
    drop(viewer);
    drop(interactive);
    daemon.shutdown().await.unwrap();
}
