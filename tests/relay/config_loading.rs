use super::*;

#[test]
fn load_does_not_create_missing_home() {
    let parent = tempdir().unwrap();
    let home = parent.path().join("missing");
    let error = relay_files::load(&home).expect_err("missing gateway files");
    assert!(!home.exists(), "load must not create the a2amx directory");
    assert!(!error.to_string().is_empty());
}

#[test]
fn load_reads_defaults_and_pairing() {
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
        0o600,
    );
    let loaded = relay_files::load(home.path()).unwrap();
    assert_eq!(loaded.guild_id, "123");
    assert_eq!(loaded.config.control_category, "a2amx");
    assert_eq!(loaded.config.control_channel, "control");
    assert_eq!(loaded.config.max_members, 2);
    assert_eq!(loaded.pairing.as_ref().unwrap().user_id, "456");
    assert_eq!(loaded.token, "bot-token");
}

#[test]
fn load_rejects_missing_or_comments_only_gateway_guild() {
    for comments in [None, Some("# gateway settings only\n")] {
        let home = tempdir().unwrap();
        let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
        layout.ensure().unwrap();
        write_file(&layout.discord_token(), "bot-token\n", 0o600);
        if let Some(comments) = comments {
            write_file(&layout.gateway_config(), comments, 0o600);
        }
        let error = relay_files::load(home.path()).unwrap_err().to_string();
        assert!(
            error.contains("gateway.toml")
                && error.contains("guild_id")
                && error.contains("not set"),
            "{error}"
        );
    }
}

#[test]
fn gateway_template_with_guild_id_loads_through_relay() {
    let home = tempdir().unwrap();
    let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    layout.ensure().unwrap();
    write_file(&layout.discord_token(), "bot-token\n", 0o600);
    a2amx::config::ensure_gateway_file(&layout).unwrap();
    let template = fs::read_to_string(layout.gateway_config()).unwrap();
    let template = template
        .lines()
        .map(|line| {
            line.strip_prefix("# guild_id = ")
                .map(|value| format!("guild_id = {value}"))
                .unwrap_or_else(|| line.to_owned())
        })
        .collect::<Vec<_>>()
        .join("\n");
    write_file(&layout.gateway_config(), &format!("{template}\n"), 0o600);
    assert_eq!(
        relay_files::load(home.path()).unwrap().guild_id,
        "123456789012345678"
    );
}
#[test]
fn load_rejects_unknown_config_key() {
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    fs::OpenOptions::new()
        .append(true)
        .open(home.path().join("config/gateway.toml"))
        .unwrap()
        .write_all(b"unknown = true\n")
        .unwrap();
    let error = relay_files::load(home.path()).unwrap_err().to_string();
    assert!(error.contains("unknown") || error.contains("gateway.toml"));
    assert!(error.contains("unknown"));
}

#[test]
fn loaded_type_is_public_contract() {
    fn assert_loaded(_: Loaded) {}
    let _ = assert_loaded;
}

#[test]
fn load_rejects_loose_modes_for_each_secret_file() {
    for file in ["discord.token", "config.toml", "pairing.json"] {
        let home = tempdir().unwrap();
        valid_gateway(home.path());
        if file == "pairing.json" {
            write_file(
                &home.path().join("state/gateway/pairing.json"),
                r#"{"guild_id":"123","user_id":"456"}"#,
                0o644,
            );
        } else {
            let path = if file == "discord.token" {
                home.path().join("config/secrets/discord.token")
            } else {
                home.path().join("config/gateway.toml")
            };
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        }
        let error = relay_files::load(home.path()).unwrap_err().to_string();
        let expected = if file == "config.toml" {
            "gateway.toml"
        } else {
            file
        };
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn load_rejects_symlinked_secret_file() {
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    let target = home.path().join("token-target");
    write_file(&target, "bot-token\n", 0o600);
    fs::remove_file(home.path().join("config/secrets/discord.token")).unwrap();
    std::os::unix::fs::symlink(&target, home.path().join("config/secrets/discord.token")).unwrap();
    let error = relay_files::load(home.path()).unwrap_err().to_string();
    assert!(error.contains("discord.token"), "{error}");
}

#[test]
fn load_rejects_bad_values_and_oversized_config() {
    for config in [
        "guild_id = \"not-numeric\"\n",
        "guild_id = \"123\"\ncontrol_channel = \"a_BAD\"\n",
        "guild_id = \"123\"\nidle = \"0ms\"\n",
    ] {
        let home = tempdir().unwrap();
        valid_gateway(home.path());
        write_file(&home.path().join("config/gateway.toml"), config, 0o600);
        assert!(relay_files::load(home.path()).is_err(), "{config}");
    }
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    write_file(
        &home.path().join("config/gateway.toml"),
        &format!("guild_id = \"123\"\n#{}\n", "x".repeat(70 * 1024)),
        0o600,
    );
    let error = relay_files::load(home.path()).unwrap_err().to_string();
    assert!(error.contains("64 KiB"), "{error}");
}

#[test]
fn load_rejects_pairing_for_another_guild() {
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"999","user_id":"456"}"#,
        0o600,
    );
    let error = relay_files::load(home.path()).unwrap_err().to_string();
    assert!(error.contains("pairing"), "{error}");
}

#[test]
fn load_rejects_symlinked_config_and_pairing() {
    for (file, target, contents) in [
        ("config.toml", "config-target", "guild_id = \"123\"\n"),
        (
            "pairing.json",
            "pairing-target",
            r#"{"guild_id":"123","user_id":"456"}"#,
        ),
    ] {
        let home = tempdir().unwrap();
        valid_gateway(home.path());
        let target_path = home.path().join(target);
        write_file(&target_path, contents, 0o600);
        let path = home.path().join(if file == "config.toml" {
            "config/gateway.toml"
        } else {
            "state/gateway/pairing.json"
        });
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        std::os::unix::fs::symlink(&target_path, &path).unwrap();
        let error = relay_files::load(home.path()).unwrap_err().to_string();
        let expected = if file == "config.toml" {
            "gateway.toml"
        } else {
            file
        };
        assert!(error.contains(expected), "{error}");
    }
}
