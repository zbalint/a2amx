use a2amx::picker::{PickerAction, PickerParser, scroll_into_view, target};
use proptest::prelude::*;

fn navigation_actions() -> impl Strategy<Value = PickerAction> {
    prop_oneof![
        (-500isize..=500).prop_map(PickerAction::Move),
        (-5isize..=5).prop_map(PickerAction::Page),
        Just(PickerAction::Home),
        Just(PickerAction::End),
    ]
}

fn other_actions() -> impl Strategy<Value = PickerAction> {
    prop_oneof![
        Just(PickerAction::Cancel),
        Just(PickerAction::Select),
        Just(PickerAction::Toggle),
        Just(PickerAction::Fold),
        Just(PickerAction::Unfold),
        Just(PickerAction::StartFilter),
        any::<u8>().prop_map(PickerAction::Char),
        Just(PickerAction::Backspace),
    ]
}
fn scroll_cases() -> impl Strategy<Value = (usize, usize, usize, usize)> {
    (0usize..=60, 0usize..=300).prop_flat_map(|(height, total)| {
        (
            Just(height),
            Just(total),
            0usize..total.max(1),
            0usize..=400,
        )
    })
}

fn visible_scroll_cases() -> impl Strategy<Value = (usize, usize, usize, usize)> {
    (1usize..=60, 0usize..=240).prop_flat_map(|(height, extra)| {
        let total = height + extra;
        (0usize..=extra).prop_flat_map(move |offset| {
            (
                Just(height),
                Just(total),
                offset..offset + height,
                Just(offset),
            )
        })
    })
}

fn other_action_cases() -> impl Strategy<Value = (usize, usize, usize, PickerAction)> {
    (1usize..=300).prop_flat_map(|total| (Just(total), 0usize..total, 0usize..=60, other_actions()))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn scroll_keeps_the_selection_visible(case in scroll_cases()) {
        let (height, total, selection, offset) = case;
        let result = scroll_into_view(offset, selection, height, total);
        if height > 0 && total > height {
            prop_assert!(result <= selection);
            prop_assert!(selection < result + height);
        } else {
            prop_assert_eq!(result, 0);
        }
    }

    #[test]
    fn scroll_does_not_jump_when_the_selection_is_already_visible(
        case in visible_scroll_cases()
    ) {
        let (height, total, selection, offset) = case;
        prop_assert_eq!(scroll_into_view(offset, selection, height, total), offset);
    }

    #[test]
    fn navigation_targets_stay_in_range(
        selection in 0usize..=400,
        height in 0usize..=60,
        total in 0usize..=300,
        action in navigation_actions(),
    ) {
        let result = target(selection, action, height, total);
        if total == 0 {
            prop_assert_eq!(result, 0);
        } else {
            prop_assert!(result < total);
            match action {
                PickerAction::Home => prop_assert_eq!(result, 0),
                PickerAction::End => prop_assert_eq!(result, total - 1),
                _ => {}
            }
        }
    }

    #[test]
    fn moves_go_the_documented_way(
        selection in 0usize..=400,
        total in 1usize..=300,
        delta in -500isize..=500,
    ) {
        let baseline = selection.min(total - 1);
        let result = target(selection, PickerAction::Move(delta), 0, total);
        if delta > 0 {
            prop_assert!(result >= baseline);
        } else if delta < 0 {
            prop_assert!(result <= baseline);
        } else {
            prop_assert_eq!(result, baseline);
        }
    }

    #[test]
    fn other_actions_leave_the_selection_alone(
        case in other_action_cases()
    ) {
        let (total, selection, height, action) = case;
        prop_assert_eq!(target(selection, action, height, total), selection);
    }

    #[test]
    fn parser_never_panics(
        bytes in prop::collection::vec(any::<u8>(), 0..=64),
        escape_alone in any::<bool>(),
        filter_mode in any::<bool>(),
    ) {
        let mut parser = PickerParser::default();
        parser.set_filter_mode(filter_mode);
        let _ = parser.feed(&bytes, escape_alone);
    }
}
