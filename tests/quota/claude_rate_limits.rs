use super::*;

#[test]
fn claude_rate_limits_convert_used_percentage_to_independent_windows() {
    let input = serde_json::json!({
        "rate_limits": {
            "five_hour": {"used_percentage": 23.7, "resets_at": 2000},
            "seven_day": {"used_percentage": 41, "resets_at": 2000}
        }
    });
    let limits = quota::from_claude_rate_limits(&input, 1000).expect("windows");
    assert_eq!(
        limits.five_hour,
        Some(quota::ClaudeWindow {
            left: 77,
            resets_at: 2000
        })
    );
    assert_eq!(
        limits.weekly,
        Some(quota::ClaudeWindow {
            left: 59,
            resets_at: 2000
        })
    );
}

fn claude_rate_limits_payload(used: serde_json::Value, resets_at: u64) -> serde_json::Value {
    serde_json::json!({
        "rate_limits": {
            "five_hour": {"used_percentage": used, "resets_at": resets_at}
        }
    })
}

#[test]
fn claude_rate_limits_conversion_covers_rounding_and_bounds() {
    for (used, expected) in [
        (serde_json::json!(23.7), 77),
        (serde_json::json!(23), 77),
        (serde_json::json!(41), 59),
        (serde_json::json!(99.9), 1),
        (serde_json::json!(100), 0),
        (serde_json::json!(0), 100),
    ] {
        let limits = quota::from_claude_rate_limits(&claude_rate_limits_payload(used, 2000), 1000)
            .expect("window");
        assert_eq!(
            limits.five_hour,
            Some(quota::ClaudeWindow {
                left: expected,
                resets_at: 2000
            })
        );
    }
    for used in [
        serde_json::json!(101),
        serde_json::json!(-1),
        serde_json::json!("23"),
    ] {
        assert_eq!(
            quota::from_claude_rate_limits(&claude_rate_limits_payload(used, 2000), 1000),
            None
        );
    }
    assert_eq!(
        quota::from_claude_rate_limits(
            &claude_rate_limits_payload(serde_json::json!(23), 1000),
            1000
        ),
        None
    );
    assert_eq!(
        quota::from_claude_rate_limits(
            &claude_rate_limits_payload(serde_json::json!(23), 999),
            1000
        ),
        None
    );
    assert_eq!(
        quota::from_claude_rate_limits(
            &claude_rate_limits_payload(serde_json::json!(23), 1000 + 691200),
            1000
        )
        .expect("boundary")
        .five_hour,
        Some(quota::ClaudeWindow {
            left: 77,
            resets_at: 692200
        })
    );
    assert_eq!(
        quota::from_claude_rate_limits(
            &claude_rate_limits_payload(serde_json::json!(23), 1000 + 691201),
            1000
        ),
        None
    );
}

#[test]
fn claude_rate_limits_windows_are_independent_and_empty_is_none() {
    let five_only = serde_json::json!({
        "rate_limits": {
            "five_hour": {"used_percentage": 23, "resets_at": 2000}
        }
    });
    let limits = quota::from_claude_rate_limits(&five_only, 1000).expect("five-hour");
    assert_eq!(
        limits.five_hour,
        Some(quota::ClaudeWindow {
            left: 77,
            resets_at: 2000
        })
    );
    assert_eq!(limits.weekly, None);
    let weekly_only = serde_json::json!({
        "rate_limits": {
            "seven_day": {"used_percentage": 41, "resets_at": 2000}
        }
    });
    let limits = quota::from_claude_rate_limits(&weekly_only, 1000).expect("weekly");
    assert_eq!(limits.five_hour, None);
    assert_eq!(
        limits.weekly,
        Some(quota::ClaudeWindow {
            left: 59,
            resets_at: 2000
        })
    );
    assert_eq!(
        quota::from_claude_rate_limits(&serde_json::json!({}), 1000),
        None
    );
    assert_eq!(
        quota::from_claude_rate_limits(
            &claude_rate_limits_payload(serde_json::json!(23), 999),
            1000
        ),
        None
    );
}
