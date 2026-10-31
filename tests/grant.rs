use std::process::Command;

use a2amx::grant;

#[test]
fn grant_table_has_one_non_default_runtime_row_per_name() {
    let expected = [
        (
            "docker",
            vec![
                "/run/docker.sock",
                "~/.docker/desktop/docker.sock",
                "~/.docker/run/docker.sock",
            ],
            vec!["DOCKER_HOST", "DOCKER_CONTEXT"],
        ),
        (
            "podman",
            vec!["/run/podman/podman.sock"],
            vec!["CONTAINER_HOST", "CONTAINER_CONNECTION"],
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
        ),
        ("crio", vec!["/run/crio/crio.sock"], vec![]),
        (
            "libvirt",
            vec!["/run/libvirt/libvirt-sock"],
            vec!["LIBVIRT_DEFAULT_URI"],
        ),
        (
            "lxd",
            vec![
                "/var/snap/lxd/common/lxd/unix.socket",
                "/var/lib/lxd/unix.socket",
                "/var/lib/incus/unix.socket",
            ],
            vec![],
        ),
        (
            "snap",
            vec!["/run/snapd.socket", "/run/snapd-snap.socket"],
            vec![],
        ),
    ];
    let all = grant::all();
    assert_eq!(all.len(), expected.len());
    for ((name, sockets, env), row) in expected.iter().zip(all) {
        assert_eq!(row.name, *name);
        assert!(!row.exposes.is_empty());
        assert!(!row.team_default_ok);
        assert_eq!(row.sockets, sockets.as_slice());
        assert_eq!(row.env, env.as_slice());
    }
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
    ] {
        let error = grant::lookup(name).expect_err("close typo");
        assert!(
            error
                .to_string()
                .contains(&format!("closest match: {closest}"))
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
fn render_list_has_every_grant_and_instruction() {
    let rendered = grant::render_list();
    let lines = rendered.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), grant::all().len() + 1);
    for (line, row) in lines.iter().zip(grant::all()) {
        assert!(
            line.starts_with(row.name),
            "{line:?} does not start with {}",
            row.name
        );
        assert!(line.contains(row.exposes));
    }
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
