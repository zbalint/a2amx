use anyhow::bail;

use crate::messaging::validate_name;

/// Derive a team name from a project directory.
pub fn team_name_for(dir: &std::path::Path, remedy: &str) -> anyhow::Result<String> {
    let component = dir
        .file_name()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cannot derive a team name from directory {}; {remedy}",
                dir.display()
            )
        })?
        .to_string_lossy();
    let mut name = String::new();
    let mut separator = false;
    for byte in component.bytes() {
        if byte.is_ascii_alphanumeric() {
            if separator && !name.is_empty() {
                name.push('-');
            }
            name.push(byte.to_ascii_lowercase() as char);
            separator = false;
        } else {
            separator = true;
        }
    }
    if name.len() > 40 {
        name.truncate(40);
        while name.ends_with('-') {
            name.pop();
        }
    }
    validate_name(&name).map_err(|error| {
        anyhow::anyhow!(
            "cannot derive a valid team name {name:?} from directory {}: {error}; {remedy}",
            dir.display()
        )
    })?;
    Ok(name)
}

/// Reject global team files in directories that are not project-like.
pub fn check_global_allowed(
    cwd: &std::path::Path,
    user_home: Option<&std::path::Path>,
    root: &std::path::Path,
) -> anyhow::Result<()> {
    let reason = home_reason(cwd, user_home)
        .or_else(|| system_reason(cwd, root))
        .or_else(|| container_reason(cwd));
    if let Some(reason) = reason {
        bail!(
            "refusing the global team file in {}: {reason}; create an a2amx.toml here, or pass --file, to start a team in this directory",
            cwd.display()
        );
    }
    Ok(())
}

fn home_reason(cwd: &std::path::Path, user_home: Option<&std::path::Path>) -> Option<&'static str> {
    if cwd.parent().is_none() {
        return Some("the filesystem root");
    }
    let home = user_home.filter(|home| home.is_absolute())?;
    if cwd == home || home.starts_with(cwd) {
        return Some("your home directory");
    }
    let below = cwd.strip_prefix(home).ok()?;
    let first = below
        .components()
        .next()
        .map(std::path::Component::as_os_str);
    if first.is_some_and(|component| component.to_string_lossy().starts_with('.')) {
        return Some("a hidden directory under your home directory");
    }
    if [
        "Desktop",
        "Documents",
        "Downloads",
        "Music",
        "Pictures",
        "Videos",
        "Public",
        "Templates",
        "Library",
        "Applications",
    ]
    .iter()
    .any(|name| below == std::path::Path::new(name))
    {
        return Some("a user-data folder");
    }
    None
}

fn system_reason(cwd: &std::path::Path, root: &std::path::Path) -> Option<&'static str> {
    let components: Vec<_> = cwd.components().collect();
    let mnt = components.get(1).is_some_and(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("mnt")
    });
    let wsl_system = mnt
        && components.get(2).is_some_and(|component| {
            let name = component.as_os_str().to_string_lossy();
            name.eq_ignore_ascii_case("wsl") || name.eq_ignore_ascii_case("wslg")
        });
    let system = [
        root,
        std::path::Path::new("/bin"),
        std::path::Path::new("/boot"),
        std::path::Path::new("/dev"),
        std::path::Path::new("/etc"),
        std::path::Path::new("/lib"),
        std::path::Path::new("/lib32"),
        std::path::Path::new("/lib64"),
        std::path::Path::new("/libx32"),
        std::path::Path::new("/proc"),
        std::path::Path::new("/run"),
        std::path::Path::new("/sbin"),
        std::path::Path::new("/sys"),
        std::path::Path::new("/usr"),
        std::path::Path::new("/snap"),
        std::path::Path::new("/nix"),
        std::path::Path::new("/lost+found"),
        std::path::Path::new("/System"),
        std::path::Path::new("/Library"),
        std::path::Path::new("/Applications"),
        std::path::Path::new("/private/etc"),
        std::path::Path::new("/var/lib"),
        std::path::Path::new("/var/log"),
        std::path::Path::new("/var/cache"),
        std::path::Path::new("/var/spool"),
        std::path::Path::new("/var/mail"),
        std::path::Path::new("/mnt/wsl"),
        std::path::Path::new("/mnt/wslg"),
    ];
    if wsl_system || system.iter().any(|prefix| cwd.starts_with(prefix)) {
        return Some("the a2amx directory or a system directory");
    }
    if components.len() >= 3 && is_mnt_drive(&components) {
        let windows = [
            "Windows",
            "Program Files",
            "Program Files (x86)",
            "ProgramData",
        ];
        if components.len() >= 4
            && windows.iter().any(|name| {
                components[3]
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(name)
            })
        {
            return Some("the a2amx directory or a system directory");
        }
    }
    None
}

fn container_reason(cwd: &std::path::Path) -> Option<&'static str> {
    fn is_direct_child(path: &std::path::Path, parent: &std::path::Path) -> bool {
        path.parent() == Some(parent)
    }
    let components: Vec<_> = cwd.components().collect();
    if [
        "/home",
        "/Users",
        "/root",
        "/tmp",
        "/var",
        "/opt",
        "/srv",
        "/mnt",
        "/media",
        "/Volumes",
        "/private",
        "/private/tmp",
        "/private/var",
    ]
    .iter()
    .any(|prefix| cwd == std::path::Path::new(prefix))
    {
        return Some("a top-level container directory");
    }
    if is_direct_child(cwd, std::path::Path::new("/home"))
        || is_direct_child(cwd, std::path::Path::new("/Users"))
    {
        return Some("another user's home directory");
    }
    if components.len() == 3 && is_mnt_drive(&components) {
        return Some("a top-level container directory");
    }
    if components.len() >= 4
        && is_mnt_drive(&components)
        && components[3]
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("Users")
        && (components.len() == 4 || components.len() == 5)
    {
        return Some("a Windows home directory");
    }
    None
}

fn is_mnt_drive(components: &[std::path::Component<'_>]) -> bool {
    components[1]
        .as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case("mnt")
        && components[2].as_os_str().to_string_lossy().len() == 1
        && components[2].as_os_str().to_string_lossy().as_bytes()[0].is_ascii_alphabetic()
}
