#[test]
fn paste_wrappers_are_unwrapped_without_changing_malformed_text() {
    use a2amx::messaging::unwrap_pastes;
    let cases = [
        (
            "\n\n<pasted_content id=\"458d\">\nalpha\nbeta\n</pasted_content id=\"458d\">\n",
            "alpha\nbeta",
        ),
        (
            "hello \n\n<pasted_content id=\"458d\">\nalpha\n</pasted_content id=\"458d\">\n",
            "hello alpha",
        ),
        (
            "a\n\n<pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">\nb\n\n<pasted_content id=\"2\">\nY\n</pasted_content id=\"2\">\n",
            "aXbY",
        ),
        (
            "\n\n<pasted_content id=\"458d\">\nalpha\n",
            "\n\n<pasted_content id=\"458d\">\nalpha\n",
        ),
        (
            "\n\n<pasted_content id=\"458d\">\nalpha\n</pasted_content id=\"zzzz\">\n",
            "\n\n<pasted_content id=\"458d\">\nalpha\n</pasted_content id=\"zzzz\">\n",
        ),
        ("plain text", "plain text"),
        (
            "é\n\n<pasted_content id=\"\">\nx\n</pasted_content id=\"\">\n",
            "é\n\n<pasted_content id=\"\">\nx\n</pasted_content id=\"\">\n",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(unwrap_pastes(input), expected);
    }
}

#[test]
fn paste_wrappers_trimmed_at_the_prompt_ends_are_still_unwrapped() {
    use a2amx::messaging::unwrap_pastes;
    // A message queued mid-turn reaches the hook with the prompt's ends trimmed.
    let cases = [
        (
            "<pasted_content id=\"7c5f\">\nalpha\n</pasted_content id=\"7c5f\">",
            "alpha",
        ),
        (
            "<pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">\n",
            "X",
        ),
        (
            "\n\n<pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">",
            "X",
        ),
        // Text glued to the tags is not a wrapper.
        (
            "hello <pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">",
            "hello <pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">",
        ),
        (
            "<pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">tail",
            "<pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">tail",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(unwrap_pastes(input), expected);
    }
}

#[test]
fn envelope_ids_require_tags_positive_bounded_digits_and_preserve_order() {
    use a2amx::messaging::envelope_ids;
    assert_eq!(
        envelope_ids("<a2amx-message id=\"m_12\" from=\"x\">"),
        vec![12]
    );
    assert_eq!(
        envelope_ids(
            "<a2amx-message id=\"m_3\"><a2amx-message id=\"m_3\"><a2amx-message id=\"m_7\">"
        ),
        vec![3, 7]
    );
    assert_eq!(
        envelope_ids(
            "<a2amx-message id=\"m_\"><a2amx-message id=\"m_x1\"><a2amx-message id=\"m_0\"> m_12"
        ),
        Vec::<i64>::new()
    );
    assert_eq!(
        envelope_ids(
            "<a2amx-message id=\"m_999999999999999999\"><a2amx-message id=\"m_1000000000000000000\">"
        ),
        vec![999_999_999_999_999_999]
    );
}

#[test]
fn restored_drafts_drop_controls_but_keep_newlines_tabs_and_unicode() {
    use a2amx::messaging::sanitize_draft;
    assert_eq!(sanitize_draft("a\u{1b}b\rc\0d\u{7f}e\u{85}f"), "abcdef");
    assert_eq!(sanitize_draft("x\ny\tz é"), "x\ny\tz é");
}

#[test]
fn team_visibility_is_symmetric_and_defaults_to_public_for_ungrouped() {
    use a2amx::messaging::{Party, TeamScope, visible};

    let private_a = TeamScope {
        name: "a".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let private_b = TeamScope {
        name: "b".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let public_a = TeamScope {
        name: "a".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let allow_a = TeamScope {
        name: "a".into(),
        private: true,
        allow: vec!["b".into()],
        agents: Vec::new(),
    };
    let allow_b = TeamScope {
        name: "b".into(),
        private: true,
        allow: vec!["a".into()],
        agents: Vec::new(),
    };
    let agent_a = TeamScope {
        name: "a2amx".into(),
        private: true,
        allow: Vec::new(),
        agents: vec!["saltmdb-architect".into()],
    };
    let agent_b = TeamScope {
        name: "saltmdb".into(),
        private: true,
        allow: Vec::new(),
        agents: vec!["a2amx-architect".into()],
    };
    let agent_a_developer = TeamScope {
        name: "a2amx".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let one_sided_a = TeamScope {
        name: "a2amx".into(),
        private: true,
        allow: Vec::new(),
        agents: vec!["saltmdb-architect".into()],
    };
    let one_sided_b = TeamScope {
        name: "saltmdb".into(),
        private: true,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    for (a, b, expected) in [
        (
            Party {
                name: None,
                team: None,
            },
            Party {
                name: None,
                team: None,
            },
            true,
        ),
        (
            Party {
                name: Some("plain"),
                team: None,
            },
            Party {
                name: Some("a"),
                team: Some(&private_a),
            },
            true,
        ),
        (
            Party {
                name: None,
                team: None,
            },
            Party {
                name: Some("b"),
                team: Some(&private_b),
            },
            true,
        ),
        (
            Party {
                name: Some("a"),
                team: Some(&public_a),
            },
            Party {
                name: Some("b"),
                team: Some(&private_b),
            },
            true,
        ),
        (
            Party {
                name: Some("a"),
                team: Some(&private_a),
            },
            Party {
                name: Some("a2"),
                team: Some(&private_a),
            },
            true,
        ),
        (
            Party {
                name: Some("a"),
                team: Some(&private_a),
            },
            Party {
                name: Some("b"),
                team: Some(&private_b),
            },
            false,
        ),
        (
            Party {
                name: Some("a"),
                team: Some(&allow_a),
            },
            Party {
                name: Some("b"),
                team: Some(&private_b),
            },
            false,
        ),
        (
            Party {
                name: Some("a"),
                team: Some(&allow_a),
            },
            Party {
                name: Some("b"),
                team: Some(&allow_b),
            },
            true,
        ),
        (
            Party {
                name: Some("a2amx-architect"),
                team: Some(&agent_a),
            },
            Party {
                name: Some("saltmdb-architect"),
                team: Some(&agent_b),
            },
            true,
        ),
        (
            Party {
                name: Some("a2amx-developer"),
                team: Some(&agent_a_developer),
            },
            Party {
                name: Some("saltmdb-architect"),
                team: Some(&agent_b),
            },
            false,
        ),
        (
            Party {
                name: Some("a2amx-architect"),
                team: Some(&one_sided_a),
            },
            Party {
                name: Some("saltmdb-architect"),
                team: Some(&one_sided_b),
            },
            false,
        ),
        (
            Party {
                name: None,
                team: Some(&agent_a),
            },
            Party {
                name: Some("saltmdb-architect"),
                team: Some(&agent_b),
            },
            false,
        ),
    ] {
        assert_eq!(visible(a, b), expected);
        assert_eq!(visible(b, a), expected);
    }
}

#[test]
fn role_validation_accepts_architect_and_rejects_invalid_shapes() {
    use a2amx::messaging::validate_role;

    assert!(validate_role("architect").is_ok());
    for role in ["", "a\nb", " architect", "architect "] {
        assert!(validate_role(role).is_err(), "{role:?}");
    }
    assert!(validate_role(&"a".repeat(65)).is_err());
}
