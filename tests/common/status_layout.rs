use a2amx::wire::{Activity, StatusInfo, TeamMember};

pub fn info(address: &str) -> StatusInfo {
    StatusInfo {
        address: address.into(),
        pending: 0,
        hold: None,
        harness: None,
        activity: None,
        confined: false,
        confine_auto: false,
        starting: false,
        quota: None,
        team: None,
        role: None,
        members: vec![],
        remote: None,
    }
}

pub fn visible(bytes: &[u8]) -> String {
    let bar = b"\x1b[24;1H";
    let Some(start) = bytes.windows(bar.len()).position(|window| window == bar) else {
        panic!("bar must draw on row 24");
    };
    let bytes = &bytes[start..];
    let mut bytes = bytes.iter().copied();
    let mut text = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte != 0x1b {
            text.push(byte);
        } else if bytes.next() == Some(b'[') {
            for byte in bytes.by_ref() {
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
        }
    }
    String::from_utf8_lossy(&text).into_owned()
}

pub fn team_member(id: &str, name: &str, activity: Option<Activity>, exited: bool) -> TeamMember {
    TeamMember {
        id: id.into(),
        name: name.into(),
        activity,
        exited,
        held: false,
        starting: false,
    }
}
