use a2amx::attach::model::{Geometry, Viewport};

#[test]
fn viewport_pty_rows_subtracts_status_and_top_rows() {
    let visible = Viewport {
        cols: 80,
        rows: 24,
        status_visible: true,
    };
    assert_eq!(visible.pty_rows(0), 22);
    assert_eq!(visible.pty_rows(2), 20);

    let hidden = Viewport {
        cols: 80,
        rows: 24,
        status_visible: false,
    };
    assert_eq!(hidden.pty_rows(2), 22);

    assert_eq!(
        Viewport {
            cols: 80,
            rows: 3,
            status_visible: true,
        }
        .pty_rows(0),
        2
    );
    assert_eq!(
        Viewport {
            cols: 80,
            rows: 2,
            status_visible: true,
        }
        .pty_rows(0),
        2
    );
    assert_eq!(
        Viewport {
            cols: 80,
            rows: 1,
            status_visible: true,
        }
        .pty_rows(0),
        1
    );
    assert_eq!(
        Viewport {
            cols: 80,
            rows: 4,
            status_visible: true,
        }
        .pty_rows(2),
        1
    );
}

#[test]
fn viewport_desired_top_follows_rows_visibility_and_team() {
    let view = |rows, status_visible| Viewport {
        cols: 80,
        rows,
        status_visible,
    };
    assert_eq!(view(24, true).desired_top(true), 2);
    assert_eq!(view(6, true).desired_top(true), 2);
    assert_eq!(view(5, true).desired_top(true), 1);
    assert_eq!(view(4, true).desired_top(true), 0);
    assert_eq!(view(24, false).desired_top(true), 0);
    assert_eq!(view(24, true).desired_top(false), 0);
}

#[test]
fn geometry_uses_the_granted_top_not_the_desired_one() {
    let viewport = Viewport {
        cols: 80,
        rows: 24,
        status_visible: true,
    };
    let refused = Geometry {
        viewport,
        in_team: true,
        granted_top: 0,
    };
    assert_eq!(refused.pty_rows(), 22);
    assert_eq!(refused.desired_top(), 2);

    let granted = Geometry {
        viewport,
        in_team: true,
        granted_top: 2,
    };
    assert_eq!(granted.pty_rows(), 20);
}
