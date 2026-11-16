use super::*;

async fn quota_for(admin: &mut Client, id: &str) -> Option<a2amx::wire::QuotaInfo> {
    match admin.request(Request::List).await.unwrap() {
        Response::Sessions { sessions } => sessions
            .into_iter()
            .find(|session| session.id == id)
            .and_then(|session| session.quota),
        other => panic!("unexpected list response: {other:?}"),
    }
}

#[tokio::test]
async fn claude_quota_reports_are_token_scoped_and_merge_independent_windows() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (claude_id, _claude_token, mut claude) = session(
        &mut admin,
        dir.path(),
        Some("agent-claude"),
        Harness::Claude,
        Deliver::Hold,
    )
    .await;
    let (omp_id, _omp_token, mut omp) = session(
        &mut admin,
        dir.path(),
        Some("agent-omp"),
        Harness::Omp,
        Deliver::Hold,
    )
    .await;
    let (screen_id, mut screen) = screen_session(&mut admin, dir.path()).await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let first_reset = now + 3600;
    let response = claude
        .request(Request::ReportQuota {
            five_hour: Some(QuotaWindow {
                left: 77,
                resets_at: first_reset,
            }),
            weekly: Some(QuotaWindow {
                left: 59,
                resets_at: first_reset,
            }),
        })
        .await
        .unwrap();
    assert_eq!(response, Response::Ok);
    let quota = quota_for(&mut admin, &claude_id)
        .await
        .expect("stored quota");
    assert_eq!(quota.five_hour_left, Some(77));
    assert_eq!(quota.weekly_left, Some(59));
    assert_eq!(
        screen
            .request(Request::ReportQuota {
                five_hour: Some(QuotaWindow {
                    left: 77,
                    resets_at: first_reset,
                }),
                weekly: Some(QuotaWindow {
                    left: 59,
                    resets_at: first_reset,
                }),
            })
            .await
            .unwrap(),
        Response::Ok
    );
    let screen_quota = quota_for(&mut admin, &screen_id)
        .await
        .expect("screen quota");
    assert_eq!(screen_quota.five_hour_left, Some(77));
    assert_eq!(screen_quota.weekly_left, Some(59));

    assert!(matches!(
        admin
            .request(Request::ReportQuota {
                five_hour: None,
                weekly: None,
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    assert!(matches!(
        claude
            .request(Request::ReportQuota {
                five_hour: Some(QuotaWindow {
                    left: 101,
                    resets_at: first_reset,
                }),
                weekly: None,
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    assert!(matches!(
        claude
            .request(Request::ReportQuota {
                five_hour: Some(QuotaWindow {
                    left: 77,
                    resets_at: now + 691200 + 120,
                }),
                weekly: None,
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));

    let second_reset = now + 3601;
    assert_eq!(
        claude
            .request(Request::ReportQuota {
                five_hour: None,
                weekly: Some(QuotaWindow {
                    left: 55,
                    resets_at: second_reset,
                }),
            })
            .await
            .unwrap(),
        Response::Ok
    );
    let quota = quota_for(&mut admin, &claude_id)
        .await
        .expect("stored quota");
    assert_eq!(quota.five_hour_left, Some(77));
    assert_eq!(quota.weekly_left, Some(55));

    assert_eq!(
        claude
            .request(Request::ReportQuota {
                five_hour: Some(QuotaWindow {
                    left: 77,
                    resets_at: now - 1,
                }),
                weekly: Some(QuotaWindow {
                    left: 59,
                    resets_at: second_reset,
                }),
            })
            .await
            .unwrap(),
        Response::Ok
    );
    let quota = quota_for(&mut admin, &claude_id)
        .await
        .expect("weekly quota");
    assert_eq!(quota.five_hour_left, None);
    assert_eq!(quota.weekly_left, Some(59));

    assert_eq!(
        omp.request(Request::ReportQuota {
            five_hour: Some(QuotaWindow {
                left: 20,
                resets_at: first_reset,
            }),
            weekly: Some(QuotaWindow {
                left: 30,
                resets_at: first_reset,
            }),
        })
        .await
        .unwrap(),
        Response::Ok
    );
    assert_eq!(quota_for(&mut admin, &omp_id).await, None);
    daemon.shutdown().await.unwrap();
}
