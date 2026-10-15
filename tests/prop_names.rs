use a2amx::messaging::validate_name;
use a2amx::names::pick;
use proptest::prelude::*;
use std::sync::LazyLock;

fn universe() -> &'static Vec<String> {
    static UNIVERSE: LazyLock<Vec<String>> = LazyLock::new(|| {
        (0..4096)
            .map(|index| {
                pick(&[], index)
                    .unwrap_or_else(|| panic!("name universe index {index} unexpectedly exhausted"))
            })
            .collect()
    });
    &UNIVERSE
}

fn taken_names() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(
        prop_oneof![
            (0..4096usize).prop_map(|index| universe()[index].clone()),
            any::<u64>().prop_map(|value| format!("arbitrary-{value}")),
        ],
        0..=50,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn seed_wraps_at_4096(seed in any::<u64>()) {
        prop_assert_eq!(pick(&[], seed), pick(&[], seed % 4096));
    }

    #[test]
    fn a_pick_is_valid_and_not_taken(seed in any::<u64>(), taken in taken_names()) {
        let name = pick(&taken, seed);
        prop_assert!(name.is_some());
        if let Some(name) = name {
            prop_assert!(!taken.iter().any(|taken_name| taken_name == &name));
            prop_assert!(validate_name(&name).is_ok());
        }
    }

}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(8))]

    #[test]
    fn one_free_name_is_always_found(index in 0..4096usize, seed in any::<u64>()) {
        let expected = universe()[index].clone();
        let taken = universe()
            .iter()
            .enumerate()
            .filter_map(|(candidate_index, name)| (candidate_index != index).then_some(name.clone()))
            .collect::<Vec<_>>();
        prop_assert_eq!(pick(&taken, seed), Some(expected));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn first_free_at_or_after_the_seed(index in 0..4096usize, run_length in 0..=40usize) {
        let taken = (0..run_length)
            .map(|offset| universe()[(index + offset) % 4096].clone())
            .collect::<Vec<_>>();
        let expected = universe()[(index + run_length) % 4096].clone();
        prop_assert_eq!(pick(&taken, index as u64), Some(expected));
    }
}

#[test]
fn name_universe_has_4096_distinct_valid_names() {
    let names = universe();
    assert_eq!(names.len(), 4096);
    for name in names {
        assert!(
            validate_name(name).is_ok(),
            "invalid generated name: {name}"
        );
    }
    let mut distinct = names.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 4096);
}

#[test]
fn all_taken_gives_none_for_boundary_seeds() {
    let taken = universe().clone();
    for seed in [0, 4095, u64::MAX] {
        assert_eq!(pick(&taken, seed), None);
    }
}
