use super::*;

#[test]
fn parses_omp_usage_windows() {
    assert_eq!(
        quota::parse_omp_usage(
            r#"{"reports":[{"provider":"openai-codex","limits":[
                {"window":{"id":"5h"},"amount":{"remainingFraction":0.97}},
                {"window":{"id":"7d"},"amount":{"remainingFraction":0.05}}
            ]}]}"#
        ),
        Some(BTreeMap::from([(
            "openai-codex".to_owned(),
            QuotaInfo {
                five_hour_left: Some(97),
                weekly_left: Some(5),
                ..Default::default()
            },
        )]))
    );
}

#[test]
fn keeps_omp_usage_providers_separate() {
    assert_eq!(
        quota::parse_omp_usage(
            r#"{"reports":[
                {"provider":"openai-codex","limits":[
                    {"window":{"id":"5h"},"amount":{"remainingFraction":1.0}},
                    {"window":{"id":"7d"},"amount":{"remainingFraction":0.85}}
                ]},
                {"provider":"opencode-go","limits":[
                    {"window":{"id":"5h"},"amount":{"remainingFraction":1.0}},
                    {"window":{"id":"7d"},"amount":{"remainingFraction":0.0}}
                ]}
            ]}"#
        ),
        Some(BTreeMap::from([
            (
                "openai-codex".to_owned(),
                QuotaInfo {
                    five_hour_left: Some(100),
                    weekly_left: Some(85),
                    ..Default::default()
                },
            ),
            (
                "opencode-go".to_owned(),
                QuotaInfo {
                    five_hour_left: Some(100),
                    weekly_left: Some(0),
                    ..Default::default()
                },
            ),
        ]))
    );
}

#[test]
fn merges_omp_usage_reports_for_one_provider() {
    assert_eq!(
        quota::parse_omp_usage(
            r#"{"reports":[
                {"provider":"openai-codex","limits":[
                    {"window":{"id":"7d"},"amount":{"remainingFraction":0.85}}
                ]},
                {"provider":"openai-codex","limits":[
                    {"window":{"id":"7d"},"amount":{"remainingFraction":0.40}}
                ]}
            ]}"#
        ),
        Some(BTreeMap::from([(
            "openai-codex".to_owned(),
            QuotaInfo {
                weekly_left: Some(40),
                ..Default::default()
            },
        )]))
    );
}

#[test]
fn rounds_and_clamps_omp_percentages() {
    assert_eq!(
        quota::parse_omp_usage(
            r#"{"reports":[{"provider":"openai-codex","limits":[
                {"window":{"id":"5h"},"amount":{"remainingFraction":0.004}},
                {"window":{"id":"7d"},"amount":{"remainingFraction":0.005}},
                {"window":{"id":"30d"},"amount":{"remainingFraction":1.5}},
                {"window":{"id":"ignored"},"amount":{"remainingFraction":-0.2}}
            ]}]}"#
        ),
        Some(BTreeMap::from([(
            "openai-codex".to_owned(),
            QuotaInfo {
                five_hour_left: Some(0),
                weekly_left: Some(1),
                ..Default::default()
            },
        )]))
    );
    assert_eq!(
        quota::parse_omp_usage(
            r#"{"reports":[{"provider":"openai-codex","limits":[
                {"window":{"id":"5h"},"amount":{"remainingFraction":1.5}},
                {"window":{"id":"7d"},"amount":{"remainingFraction":-0.2}}
            ]}]}"#
        ),
        Some(BTreeMap::from([(
            "openai-codex".to_owned(),
            QuotaInfo {
                five_hour_left: Some(100),
                weekly_left: Some(0),
                ..Default::default()
            },
        )]))
    );
}

#[test]
fn omp_usage_may_report_one_window() {
    assert_eq!(
        quota::parse_omp_usage(
            r#"{"reports":[{"provider":"openai-codex","limits":[
                {"window":{"id":"5h"},"amount":{"remainingFraction":0.40}}
            ]}]}"#
        ),
        Some(BTreeMap::from([(
            "openai-codex".to_owned(),
            QuotaInfo {
                five_hour_left: Some(40),
                ..Default::default()
            },
        )]))
    );
}

#[test]
fn omp_usage_without_supported_windows_or_provider_is_none() {
    for json in [
        "",
        "not json",
        "{}",
        r#"{"reports":[]}"#,
        r#"{"reports":[{"provider":"openai-codex","limits":[{"window":{"id":"30d"},"amount":{"remainingFraction":0.5}}]}]}"#,
        r#"{"reports":[{"limits":[{"window":{"id":"5h"},"amount":{"remainingFraction":0.5}}]}]}"#,
        r#"{"reports":[{"provider":42,"limits":[{"window":{"id":"5h"},"amount":{"remainingFraction":0.5}}]}]}"#,
    ] {
        assert_eq!(quota::parse_omp_usage(json), None, "fixture: {json:?}");
    }
}

#[test]
fn omp_usage_ignores_identity_fields() {
    assert_eq!(
        quota::parse_omp_usage(
            r#"{
                "email":"fictional@example.invalid",
                "accountId":"fictional-account",
                "metadata":{"organisationId":"fictional-org"},
                "reports":[{"provider":"fictional-provider","limits":[
                    {"window":{"id":"5h"},"amount":{"remainingFraction":0.97}},
                    {"window":{"id":"7d"},"amount":{"remainingFraction":0.05}}
                ]}]
            }"#
        ),
        Some(BTreeMap::from([(
            "fictional-provider".to_owned(),
            QuotaInfo {
                five_hour_left: Some(97),
                weekly_left: Some(5),
                ..Default::default()
            },
        )]))
    );
}
