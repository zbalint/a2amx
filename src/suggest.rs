#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem {
    Rust,
    Node,
    Go,
    Maven,
    Gradle,
    Python,
}

struct EcosystemInfo {
    ecosystem: Ecosystem,
    label: &'static str,
    markers: &'static [&'static str],
    write_paths: &'static [&'static str],
    note: &'static str,
}

const ECOSYSTEMS: [EcosystemInfo; 6] = [
    EcosystemInfo {
        ecosystem: Ecosystem::Rust,
        label: "Rust",
        markers: &["Cargo.toml"],
        write_paths: &["~/.cargo/registry"],
        note: "Rust: a Git dependency also needs ~/.cargo/git (untested).",
    },
    EcosystemInfo {
        ecosystem: Ecosystem::Node,
        label: "Node",
        markers: &["package.json"],
        write_paths: &["~/.npm"],
        note: "Node: global installs and version-manager directories are not covered.",
    },
    EcosystemInfo {
        ecosystem: Ecosystem::Go,
        label: "Go",
        markers: &["go.mod"],
        write_paths: &[],
        note: "Go: run go env GOMODCACHE GOCACHE and add both paths as write entries; do not open ~/go/bin.",
    },
    EcosystemInfo {
        ecosystem: Ecosystem::Maven,
        label: "Maven",
        markers: &["pom.xml"],
        write_paths: &["~/.m2/repository"],
        note: "Maven: ~/.m2/settings.xml can hold passwords; add hide = [\"~/.m2/settings.xml\"] when the agent does not need it.",
    },
    EcosystemInfo {
        ecosystem: Ecosystem::Gradle,
        label: "Gradle",
        markers: &[
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ],
        write_paths: &["~/.gradle/caches", "~/.gradle/wrapper"],
        note: "Gradle: gradle.properties and init.d stay closed; do not open all of ~/.gradle.",
    },
    EcosystemInfo {
        ecosystem: Ecosystem::Python,
        label: "Python",
        markers: &["pyproject.toml", "requirements.txt"],
        write_paths: &[],
        note: "Python: a virtual environment inside the workspace needs no write entry.",
    },
];

/// Which ecosystems have a marker file, each with the first marker found.
pub fn detect(has_file: impl Fn(&str) -> bool) -> Vec<(Ecosystem, &'static str)> {
    ECOSYSTEMS
        .iter()
        .filter_map(|info| {
            info.markers
                .iter()
                .copied()
                .find(|marker| has_file(marker))
                .map(|marker| (info.ecosystem, marker))
        })
        .collect()
}
/// Return the unique writable cache paths for the detected ecosystems.
pub fn write_paths(found: &[(Ecosystem, &'static str)]) -> Vec<&'static str> {
    let mut write_paths = Vec::new();
    for info in &ECOSYSTEMS {
        if found
            .iter()
            .any(|(ecosystem, _)| *ecosystem == info.ecosystem)
        {
            for path in info.write_paths {
                if !write_paths.contains(path) {
                    write_paths.push(*path);
                }
            }
        }
    }
    write_paths
}

/// Render the confinement fragment and its ecosystem notes.
pub fn render(found: &[(Ecosystem, &'static str)]) -> String {
    if found.is_empty() {
        return "# No known build files here. The workspace itself needs no write entry.\n# Run the tool once, read the failing path, and see docs/confinement.md, Recipes by ecosystem.\n".to_owned();
    }

    let mut output = String::new();
    output.push_str("# Found: ");
    for (index, (ecosystem, marker)) in found.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        output.push_str(marker);
        output.push_str(" (");
        if let Some(info) = info_for(*ecosystem) {
            output.push_str(info.label);
        }
        output.push(')');
    }
    output.push('\n');

    let write_paths = write_paths(found);
    if !write_paths.is_empty() {
        output.push_str("# Add this to the session's table in your team file, or pass --confine-write for each path. Confinement must be enabled (enabled = true here or in the team's [confine] table).\n");
        output.push_str("[session.confine]\nwrite = [");
        for (index, path) in write_paths.iter().enumerate() {
            if index > 0 {
                output.push_str(", ");
            }
            output.push('\"');
            output.push_str(path);
            output.push('\"');
        }
        output.push_str("]\n");
    }

    for info in &ECOSYSTEMS {
        if found
            .iter()
            .any(|(ecosystem, _)| *ecosystem == info.ecosystem)
        {
            output.push_str("# ");
            output.push_str(info.note);
            output.push('\n');
        }
    }
    output.push_str("# More: docs/confinement.md, Recipes by ecosystem\n");
    output
}

fn info_for(ecosystem: Ecosystem) -> Option<&'static EcosystemInfo> {
    ECOSYSTEMS.iter().find(|info| info.ecosystem == ecosystem)
}
