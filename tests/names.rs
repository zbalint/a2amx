use a2amx::messaging::validate_name;
use a2amx::names::pick;

#[test]
fn picks_expected_names_from_seed_indices() {
    assert_eq!(pick(&[], 0), Some("soggy-walrus".to_owned()));
    assert_eq!(pick(&[], 1), Some("soggy-waffle".to_owned()));
    assert_eq!(pick(&[], 64), Some("grumpy-walrus".to_owned()));
    assert_eq!(pick(&[], 4095), Some("dusty-raccoon".to_owned()));
}

#[test]
fn seed_wraps_and_taken_names_probe_forward() {
    assert_eq!(pick(&[], 4096), Some("soggy-walrus".to_owned()));
    assert_eq!(
        pick(&["soggy-walrus".to_owned()], 0),
        Some("soggy-waffle".to_owned())
    );
    assert_eq!(
        pick(&["dusty-raccoon".to_owned()], 4095),
        Some("soggy-walrus".to_owned())
    );
}

#[test]
fn every_combination_is_valid_and_exhaustion_returns_none() {
    let mut taken = Vec::with_capacity(4096);
    for seed in 0..4096 {
        let name = pick(&taken, seed).expect("a free generated name");
        validate_name(&name).expect("generated name is valid");
        taken.push(name);
    }
    assert_eq!(taken.len(), 4096);
    assert_eq!(pick(&taken, 0), None);
    assert_eq!(pick(&taken, 1234), None);
}
