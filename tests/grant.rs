use std::process::Command;

use a2amx::{grant, team};

#[test]
fn grant_table_has_expected_rows() {
    let expected = [
        (
            "docker",
            vec![
                "/run/docker.sock",
                "~/.docker/desktop/docker.sock",
                "~/.docker/run/docker.sock",
            ],
            vec!["DOCKER_HOST", "DOCKER_CONTEXT"],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "podman",
            vec!["/run/podman/podman.sock"],
            vec!["CONTAINER_HOST", "CONTAINER_CONNECTION"],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "containerd",
            vec![
                "/run/containerd/containerd.sock",
                "/run/containerd/containerd.sock.ttrpc",
                "/run/buildkit/buildkitd.sock",
                "/run/k3s/containerd/containerd.sock",
            ],
            vec!["CONTAINERD_ADDRESS", "BUILDKIT_HOST"],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "crio",
            vec!["/run/crio/crio.sock"],
            vec![],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "libvirt",
            vec!["/run/libvirt/libvirt-sock"],
            vec!["LIBVIRT_DEFAULT_URI"],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "lxd",
            vec![
                "/var/snap/lxd/common/lxd/unix.socket",
                "/var/lib/lxd/unix.socket",
                "/var/lib/incus/unix.socket",
            ],
            vec![],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "snap",
            vec!["/run/snapd.socket", "/run/snapd-snap.socket"],
            vec![],
            vec![],
            vec![],
            false,
            false,
        ),
        (
            "harness-config",
            vec![],
            vec![],
            vec![],
            vec![],
            false,
            true,
        ),
        (
            "gh",
            vec![],
            vec!["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"],
            vec!["~/.config/gh"],
            vec![],
            true,
            false,
        ),
        (
            "git-credentials",
            vec![],
            vec![],
            vec!["~/.git-credentials", "~/.config/git/credentials"],
            vec![],
            true,
            false,
        ),
        (
            "ssh",
            vec![],
            vec![],
            vec!["~/.ssh"],
            vec!["/etc/ssh/ssh_config"],
            true,
            false,
        ),
    ];
    let all = grant::all();
    assert_eq!(all.len(), expected.len());
    for ((name, sockets, env, unhides, stand_ins, team_default_ok, own_config), row) in
        expected.iter().zip(all)
    {
        assert_eq!(row.name, *name);
        assert!(!row.exposes.is_empty());
        assert_eq!(row.team_default_ok, *team_default_ok);
        assert_eq!(row.sockets, sockets.as_slice());
        assert_eq!(row.env, env.as_slice());
        assert_eq!(row.unhides, unhides.as_slice());
        assert_eq!(row.stand_ins, stand_ins.as_slice());
        assert_eq!(row.own_config, *own_config);
    }
}

#[test]
fn credential_grant_rows_have_exact_exposure_text() {
    let gh = grant::lookup("gh").expect("gh grant");
    assert_eq!(
        gh.exposes,
        "your GitHub CLI login (~/.config/gh) and its tokens in the environment: the agent can push, open and merge pull requests and read private repositories as you"
    );
    let git_credentials = grant::lookup("git-credentials").expect("git-credentials grant");
    assert_eq!(
        git_credentials.exposes,
        "the plain-text git credential files: every host and password stored there"
    );
    let ssh = grant::lookup("ssh").expect("ssh grant");
    assert_eq!(
        ssh.exposes,
        "your ssh keys and config (~/.ssh, read-only): the agent can log in to every host those keys open as you, and push with them"
    );
}

#[test]
fn grant_validation_rejects_repeats_and_suggests_close_names() {
    let repeated =
        grant::validate(&["docker".into(), "docker".into()]).expect_err("repeated grant");
    assert!(repeated.to_string().contains("repeated"));

    for (name, closest) in [
        ("container", "containerd"),
        ("containrs", "containerd"),
        ("contaner", "containerd"),
        ("gx", "gh"),
        ("g", "gh"),
        ("gj", "gh"),
        ("lx", "lxd"),
        ("gitcredentials", "git-credentials"),
    ] {
        let error = grant::lookup(name).expect_err("close typo");
        assert!(
            error
                .to_string()
                .contains(&format!("closest match: {closest}"))
        );
    }
    for name in ["gpg", "git"] {
        let error = grant::lookup(name).expect_err("ambiguous close typo");
        assert!(
            !error.to_string().contains("closest match"),
            "{name} unexpectedly received a suggestion: {error}"
        );
    }
    let distant = grant::lookup("zzz").expect_err("unknown grant");
    assert!(distant.to_string().contains("docker"));
    assert!(!distant.to_string().contains("closest"));
}

#[test]
fn old_containers_name_reports_the_split_grants() {
    let error = grant::lookup("containers").expect_err("old grant name");
    assert_eq!(
        error.to_string(),
        "confinement grant \"containers\" was split into one grant per runtime: docker, podman, containerd, crio, libvirt, lxd, snap; run a2amx confine list"
    );
}
#[test]
fn team_table_accepts_credential_grants_and_rejects_the_rest() {
    for grant in ["gh", "ssh", "git-credentials"] {
        team::parse(&format!(
            "[confine]\ngrants = ['{grant}']\n[[session]]\nname = 'worker'\ncommand = ['cat']"
        ))
        .expect("credential grant is allowed team-wide");
    }
    for grant in ["docker", "harness-config"] {
        let error = team::parse(&format!(
            "[confine]\ngrants = ['{grant}']\n[[session]]\nname = 'worker'\ncommand = ['cat']"
        ))
        .expect_err("non-credential grant is session-only");
        assert!(error.to_string().contains("grant it in the session"));
    }
}

#[test]
fn render_list_has_every_grant_and_instruction() {
    let rendered = grant::render_list();
    let lines = rendered.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), grant::all().len() + 1);
    for (line, row) in lines.iter().zip(grant::all()) {
        let after_name = line
            .strip_prefix(row.name)
            .expect("each rendered row starts with its grant name");
        assert_eq!(
            after_name.chars().next(),
            Some(' '),
            "{line:?} has no separating space after {}",
            row.name
        );
        let scope = if row.team_default_ok {
            "team or session"
        } else {
            "session only"
        };
        let scope_offset = after_name
            .find(scope)
            .expect("each rendered row reports its grant scope");
        assert!(
            scope_offset > 0
                && after_name[..scope_offset]
                    .chars()
                    .all(|character| character.is_whitespace()),
            "{line:?} does not separate the name and scope with whitespace"
        );
        assert!(line.contains(row.exposes));
    }
    assert_eq!(
        lines[8],
        "gh              team or sessionyour GitHub CLI login (~/.config/gh) and its tokens in the environment: the agent can push, open and merge pull requests and read private repositories as you"
    );
    assert_eq!(
        lines[9],
        "git-credentials team or sessionthe plain-text git credential files: every host and password stored there"
    );
    assert_eq!(
        lines[10],
        "ssh             team or sessionyour ssh keys and config (~/.ssh, read-only): the agent can log in to every host those keys open as you, and push with them"
    );
    assert_eq!(
        lines.last().copied(),
        Some(
            "Give a grant with --confine-grant NAME, or grants = [\"NAME\"] in [session.confine]."
        )
    );
}

#[test]
fn binary_confine_list_needs_no_home_or_daemon() {
    let output = Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .env_clear()
        .args(["confine", "list"])
        .output()
        .expect("run confine list");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, grant::render_list().as_bytes());
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
}
