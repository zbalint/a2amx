mod common;

use std::path::Path;
use std::process::{Command, Stdio};

use a2amx::harness::PEER_AUTHORIZATION_PROMPT;

async fn capture_launch(
    home: &Path,
    harness: &str,
    authorize: bool,
) -> anyhow::Result<(Vec<String>, String)> {
    let args_path = home.join("arguments");
    let bin_path = home.join("binary");
    let home = home.to_path_buf();
    let harness = harness.to_owned();
    let output = tokio::task::spawn_blocking({
        let args_path = args_path.clone();
        let bin_path = bin_path.clone();
        move || {
            let mut command = Command::new(env!("CARGO_BIN_EXE_a2amx"));
            command.arg("--home").arg(home).args(["new", "--detach", "--harness", &harness]);
            if !authorize {
                command.arg("--no-authorize-peers");
            }
            command.args(["--", "sh", "-c", "printf '%s\\n' \"$@\" >\"$ARGS\"; printf '%s' \"${A2AMX_BIN-unset}\" >\"$BIN\"; sleep 30", "sh"])
                .env("ARGS", args_path).env("BIN", bin_path)
                .env_remove("A2AMX_BIN").stdin(Stdio::null()).output()
        }
    }).await??;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(common::eventually(|| async {
        let args = std::fs::read_to_string(&args_path).ok()?;
        let bin = std::fs::read_to_string(&bin_path).ok()?;
        (!bin.is_empty()).then(|| (args.lines().map(str::to_owned).collect(), bin))
    })
    .await)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn omp_new_wires_the_extension_overlay_and_operator_line() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (args, bin) = capture_launch(dir.path(), "omp", true).await?;
    assert_eq!(
        args,
        vec![
            "-e".to_owned(),
            dir.path()
                .join(format!("omp/extension-{}.ts", env!("CARGO_PKG_VERSION")))
                .to_string_lossy()
                .into_owned(),
            "--config".to_owned(),
            dir.path()
                .join("omp/overlay.yml")
                .to_string_lossy()
                .into_owned(),
            "--append-system-prompt".to_owned(),
            PEER_AUTHORIZATION_PROMPT.to_owned(),
        ]
    );
    assert_eq!(
        bin,
        std::fs::canonicalize(env!("CARGO_BIN_EXE_a2amx"))
            .unwrap()
            .to_string_lossy()
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_authorize_peers_leaves_the_operator_line_out() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (args, _) = capture_launch(dir.path(), "omp", false).await?;
    assert_eq!(
        args,
        vec![
            "-e".to_owned(),
            dir.path()
                .join(format!("omp/extension-{}.ts", env!("CARGO_PKG_VERSION")))
                .to_string_lossy()
                .into_owned(),
            "--config".to_owned(),
            dir.path()
                .join("omp/overlay.yml")
                .to_string_lossy()
                .into_owned(),
        ]
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generic_sessions_get_no_a2amx_bin() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (_, bin) = capture_launch(dir.path(), "generic", true).await?;
    assert_eq!(bin, "unset");
    Ok(())
}

#[test]
fn install_writes_owner_only_files_and_repairs_edits() {
    use std::os::unix::fs::PermissionsExt;

    let home = tempfile::tempdir().unwrap();
    let installed = a2amx::omp::install(home.path()).unwrap();
    assert!(installed.extension.is_absolute());
    assert!(installed.overlay.is_absolute());
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&home.path().join("omp")), 0o700);
    assert_eq!(mode(&installed.extension), 0o600);
    assert_eq!(mode(&installed.overlay), 0o600);
    assert_eq!(
        std::fs::read_to_string(&installed.extension).unwrap(),
        a2amx::omp::render_extension().unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(&installed.overlay).unwrap(),
        "tools:\n  xdev: false\n"
    );
    std::fs::write(&installed.extension, "x").unwrap();
    std::fs::set_permissions(&installed.extension, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(&installed.overlay, std::fs::Permissions::from_mode(0o644)).unwrap();
    let repaired = a2amx::omp::install(home.path()).unwrap();
    assert_eq!(repaired.extension, installed.extension);
    assert_eq!(repaired.overlay, installed.overlay);
    assert_eq!(
        std::fs::read_to_string(&repaired.extension).unwrap(),
        a2amx::omp::render_extension().unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(&repaired.overlay).unwrap(),
        "tools:\n  xdev: false\n"
    );
    assert_eq!(mode(&repaired.extension), 0o600);
    assert_eq!(mode(&repaired.overlay), 0o600);
}

#[test]
fn the_rendered_extension_embeds_the_tool_schemas_and_the_protocol() {
    let rendered = a2amx::omp::render_extension().unwrap();
    for name in ["\"list_agents\"", "\"send_message\"", "\"message_status\""] {
        assert!(rendered.contains(name));
    }
    assert!(rendered.contains("Returns a message id once the message is accepted"));
    assert!(!rendered.contains("[/* a2amx:tools */]"));
    assert!(rendered.contains(&format!(
        "const PROTOCOL = {};",
        a2amx::wire::BRIDGE_PROTOCOL
    )));
}

#[test]
fn wire_omp_argv_inserts_before_a_separator_and_honors_authorize_peers() {
    let argv = || vec!["omp".to_owned(), "--".to_owned(), "hello".to_owned()];
    assert_eq!(
        a2amx::harness::wire_omp_argv(argv(), Path::new("E"), Path::new("O"), true),
        vec![
            "omp",
            "-e",
            "E",
            "--config",
            "O",
            "--append-system-prompt",
            PEER_AUTHORIZATION_PROMPT,
            "--",
            "hello"
        ],
    );
    assert_eq!(
        a2amx::harness::wire_omp_argv(argv(), Path::new("E"), Path::new("O"), false),
        vec!["omp", "-e", "E", "--config", "O", "--", "hello"],
    );
}
