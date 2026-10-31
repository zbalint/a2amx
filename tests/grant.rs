use a2amx::grant;

#[test]
fn containers_is_a_non_team_default_grant() {
    let all = grant::all();
    let containers = grant::lookup("containers").expect("containers grant");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, containers.name);
    assert_eq!(all[0].exposes, containers.exposes);
    assert!(!containers.team_default_ok);
    assert!(containers.exposes.contains("effectively root"));
}

#[test]
fn grant_validation_rejects_repeats_and_suggests_close_names() {
    let repeated =
        grant::validate(&["containers".into(), "containers".into()]).expect_err("repeated grant");
    assert!(repeated.to_string().contains("repeated"));

    let close = grant::lookup("container").expect_err("close typo");
    assert!(close.to_string().contains("closest match: containers"));

    let near = grant::lookup("containrs").expect_err("near typo");
    assert!(near.to_string().contains("closest match: containers"));

    let boundary = grant::lookup("contaner").expect_err("distance-two typo");
    assert!(boundary.to_string().contains("closest match: containers"));

    let distant = grant::lookup("zzz").expect_err("unknown grant");
    assert!(distant.to_string().contains("containers"));
    assert!(!distant.to_string().contains("closest"));
}
