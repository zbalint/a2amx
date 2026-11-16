use super::*;

#[test]
fn agy_status_selects_model_group_windows() {
    let input = serde_json::json!({
        "model": {"id": "gemini-3.8-flash-low"},
        "quota": {
            "3p-5h": {"remaining_fraction": 0.4, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 100},
            "3p-weekly": {"remaining_fraction": 0.9, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 200},
            "gemini-5h": {"remaining_fraction": 0.763, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 300},
            "gemini-weekly": {"remaining_fraction": 0.9, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 400},
            "gemini-argon-wtu-weekly": {"remaining_fraction": 0.8, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 500},
            "ignored": {"remaining_fraction": 0.01, "reset_in_seconds": 1}
        }
    });
    assert_eq!(
        quota::from_agy_status(&input, 1000),
        Some(quota::ClaudeRateLimits {
            five_hour: Some(quota::ClaudeWindow {
                left: 77,
                resets_at: 1300
            }),
            weekly: Some(quota::ClaudeWindow {
                left: 90,
                resets_at: 1400
            }),
        })
    );
}

#[test]
fn agy_status_display_claude_selects_model_group_windows() {
    let input = serde_json::json!({
        "model": {"id": "Claude Opus 5.5 (Low)"},
        "quota": {
            "3p-5h": {"remaining_fraction": 0.4, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 100},
            "3p-weekly": {"remaining_fraction": 0.9, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 200},
            "gemini-5h": {"remaining_fraction": 0.763, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 300},
            "gemini-weekly": {"remaining_fraction": 0.9, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 400},
            "gemini-argon-wtu-weekly": {"remaining_fraction": 0.8, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 500},
            "ignored": {"remaining_fraction": 0.01, "reset_in_seconds": 1}
        }
    });
    assert_eq!(
        quota::from_agy_status(&input, 1000),
        Some(quota::ClaudeRateLimits {
            five_hour: Some(quota::ClaudeWindow {
                left: 40,
                resets_at: 1100
            }),
            weekly: Some(quota::ClaudeWindow {
                left: 90,
                resets_at: 1200
            }),
        })
    );
}

#[test]
fn agy_status_display_gpt_selects_model_group_windows() {
    let input = serde_json::json!({
        "model": {"id": "GPT-OSS 120B (Medium)"},
        "quota": {
            "3p-5h": {"remaining_fraction": 0.4, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 100},
            "3p-weekly": {"remaining_fraction": 0.9, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 200},
            "gemini-5h": {"remaining_fraction": 0.763, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 300},
            "gemini-weekly": {"remaining_fraction": 0.9, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 400},
            "gemini-argon-wtu-weekly": {"remaining_fraction": 0.8, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 500},
            "ignored": {"remaining_fraction": 0.01, "reset_in_seconds": 1}
        }
    });
    assert_eq!(
        quota::from_agy_status(&input, 1000),
        Some(quota::ClaudeRateLimits {
            five_hour: Some(quota::ClaudeWindow {
                left: 40,
                resets_at: 1100
            }),
            weekly: Some(quota::ClaudeWindow {
                left: 90,
                resets_at: 1200
            }),
        })
    );
}

#[test]
fn agy_status_display_argon_selects_model_group_windows() {
    let input = serde_json::json!({
        "model": {"id": "Gemini 4 Argon (Medium)"},
        "quota": {
            "3p-5h": {"remaining_fraction": 0.4, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 100},
            "3p-weekly": {"remaining_fraction": 0.9, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 200},
            "gemini-5h": {"remaining_fraction": 0.763, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 300},
            "gemini-weekly": {"remaining_fraction": 0.9, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 400},
            "gemini-argon-wtu-weekly": {"remaining_fraction": 0.8, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 500},
            "ignored": {"remaining_fraction": 0.01, "reset_in_seconds": 1}
        }
    });
    assert_eq!(
        quota::from_agy_status(&input, 1000),
        Some(quota::ClaudeRateLimits {
            five_hour: None,
            weekly: Some(quota::ClaudeWindow {
                left: 80,
                resets_at: 1500
            }),
        })
    );
}

#[test]
fn agy_status_display_gemini_selects_model_group_windows() {
    let input = serde_json::json!({
        "model": {"id": "Gemini 3.8 Flash (Medium)"},
        "quota": {
            "3p-5h": {"remaining_fraction": 0.4, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 100},
            "3p-weekly": {"remaining_fraction": 0.9, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 200},
            "gemini-5h": {"remaining_fraction": 0.763, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 300},
            "gemini-weekly": {"remaining_fraction": 0.9, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 400},
            "gemini-argon-wtu-weekly": {"remaining_fraction": 0.8, "reset_time": "2026-11-14T20:00:00Z", "reset_in_seconds": 500},
            "ignored": {"remaining_fraction": 0.01, "reset_in_seconds": 1}
        }
    });
    assert_eq!(
        quota::from_agy_status(&input, 1000),
        Some(quota::ClaudeRateLimits {
            five_hour: Some(quota::ClaudeWindow {
                left: 77,
                resets_at: 1300
            }),
            weekly: Some(quota::ClaudeWindow {
                left: 90,
                resets_at: 1400
            }),
        })
    );
}

#[test]
fn agy_status_unknown_models_choose_pessimistic_windows_and_known_missing_stays_missing() {
    let input = serde_json::json!({
        "model": {"id": "foo"},
        "quota": {
            "3p-5h": {"remaining_fraction": 0.4, "reset_in_seconds": 100},
            "gemini-5h": {"remaining_fraction": 0.6, "reset_in_seconds": 300},
            "3p-weekly": {"remaining_fraction": 0.9, "reset_in_seconds": 400},
            "gemini-weekly": {"remaining_fraction": 0.9, "reset_in_seconds": 200}
        }
    });
    assert_eq!(
        quota::from_agy_status(&input, 1000),
        Some(quota::ClaudeRateLimits {
            five_hour: Some(quota::ClaudeWindow {
                left: 40,
                resets_at: 1100
            }),
            weekly: Some(quota::ClaudeWindow {
                left: 90,
                resets_at: 1200
            }),
        })
    );
    let known_missing = serde_json::json!({
        "model": {"id": "claude-sonnet-x"},
        "quota": {"gemini-5h": {"remaining_fraction": 0.1, "reset_in_seconds": 100}}
    });
    assert_eq!(quota::from_agy_status(&known_missing, 1000), None);
}

#[test]
fn agy_status_selects_claude_and_argon_groups_and_missing_model_is_pessimistic() {
    let claude = serde_json::json!({
        "model": {"id": "claude-sonnet-x"},
        "quota": {
            "3p-5h": {"remaining_fraction": 0.4, "reset_in_seconds": 100},
            "3p-weekly": {"remaining_fraction": 0.8, "reset_in_seconds": 200},
            "gemini-5h": {"remaining_fraction": 0.1, "reset_in_seconds": 300},
            "gemini-weekly": {"remaining_fraction": 0.1, "reset_in_seconds": 400}
        }
    });
    assert_eq!(
        quota::from_agy_status(&claude, 1000),
        Some(quota::ClaudeRateLimits {
            five_hour: Some(quota::ClaudeWindow {
                left: 40,
                resets_at: 1100
            }),
            weekly: Some(quota::ClaudeWindow {
                left: 80,
                resets_at: 1200
            }),
        })
    );
    let argon = serde_json::json!({
        "model": {"id": "gemini-argon-pro"},
        "quota": {
            "gemini-5h": {"remaining_fraction": 0.1, "reset_in_seconds": 100},
            "gemini-argon-wtu-weekly": {"remaining_fraction": 0.8, "reset_in_seconds": 200}
        }
    });
    assert_eq!(
        quota::from_agy_status(&argon, 1000),
        Some(quota::ClaudeRateLimits {
            five_hour: None,
            weekly: Some(quota::ClaudeWindow {
                left: 80,
                resets_at: 1200
            }),
        })
    );
    let missing_model = serde_json::json!({
        "quota": {
            "3p-weekly": {"remaining_fraction": 0.9, "reset_in_seconds": 400},
            "gemini-weekly": {"remaining_fraction": 0.9, "reset_in_seconds": 200}
        }
    });
    assert_eq!(
        quota::from_agy_status(&missing_model, 1000)
            .expect("weekly fallback")
            .weekly,
        Some(quota::ClaudeWindow {
            left: 90,
            resets_at: 1200
        })
    );
}

#[test]
fn agy_status_rejects_invalid_buckets_and_rounds_remaining_fraction() {
    for (fraction, expected) in [
        (serde_json::json!(0.763), 77),
        (serde_json::json!(0.07), 7),
        (serde_json::json!(0.001), 1),
        (serde_json::json!(0.0), 0),
        (serde_json::json!(1e-10), 1),
        (serde_json::json!(1e-12), 0),
    ] {
        let input = serde_json::json!({
            "model": {"id": "gemini-x"},
            "quota": {"gemini-5h": {
                "remaining_fraction": fraction,
                "reset_in_seconds": 12
            }}
        });
        assert_eq!(
            quota::from_agy_status(&input, 1000)
                .expect("window")
                .five_hour
                .expect("five-hour")
                .left,
            expected
        );
    }
    for bucket in [
        serde_json::json!({"remaining_fraction": 0.5, "reset_in_seconds": 0}),
        serde_json::json!({"remaining_fraction": 0.5, "reset_in_seconds": 700000}),
        serde_json::json!({"remaining_fraction": 1.2, "reset_in_seconds": 12}),
        serde_json::json!({"remaining_fraction": "x", "reset_in_seconds": 12}),
    ] {
        let input = serde_json::json!({
            "model": {"id": "gemini-x"},
            "quota": {"gemini-5h": bucket}
        });
        assert_eq!(quota::from_agy_status(&input, 1000), None);
    }
    assert_eq!(
        quota::from_agy_status(&serde_json::json!({"model": {"id": "gemini-x"}}), 1000),
        None
    );
    assert_eq!(
        quota::from_agy_status(
            &serde_json::json!({"model": {"id": "gemini-x"}, "quota": 5}),
            1000
        ),
        None
    );
}
