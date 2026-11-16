use super::*;

#[test]
fn exhausted_names_the_windows_at_zero() {
    let exhausted = |five_hour_left, weekly_left| {
        QuotaInfo {
            five_hour_left,
            weekly_left,
            ..Default::default()
        }
        .exhausted()
    };
    assert_eq!(exhausted(Some(0), Some(0)), Some("5h and weekly"));
    assert_eq!(exhausted(Some(0), Some(5)), Some("5h"));
    assert_eq!(exhausted(Some(5), Some(0)), Some("weekly"));
    assert_eq!(exhausted(Some(5), None), None);
    assert_eq!(
        QuotaInfo {
            limit_reached: true,
            ..Default::default()
        }
        .exhausted(),
        Some("usage")
    );
    assert_eq!(
        QuotaInfo {
            limit_reached: true,
            five_hour_left: Some(0),
            ..Default::default()
        }
        .exhausted(),
        Some("5h")
    );
    assert_eq!(
        QuotaInfo {
            limit_reached: false,
            ..Default::default()
        }
        .exhausted(),
        None
    );
}

#[test]
fn cell_preserves_screen_limit_with_windows() {
    assert_eq!(
        quota::cell(Some(QuotaInfo {
            five_hour_left: Some(97),
            weekly_left: Some(5),
            limit_reached: true,
        })),
        "5h 97% wk 5% limit"
    );
    assert_eq!(
        quota::cell(Some(QuotaInfo {
            limit_reached: true,
            ..Default::default()
        })),
        "limit"
    );
}
