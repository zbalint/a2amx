use a2amx::mouse::{Shifted, parse_report, shift_row};

#[test]
fn mouse_reports_shift_or_claim_reserved_rows() {
    let report = parse_report(b"\x1b[<0;5;1M", 0).unwrap();
    assert!(matches!(shift_row(&report, 1), Shifted::Claimed(r) if r.column == 5 && r.row == 1));
    for (input, expected) in [
        (b"\x1b[<0;5;2M".as_slice(), b"\x1b[<0;5;1M".as_slice()),
        (b"\x1b[<64;10;24m".as_slice(), b"\x1b[<64;10;23m".as_slice()),
        (b"\x1b[<32;005;2M".as_slice(), b"\x1b[<32;005;1M".as_slice()),
        (
            b"\x1b[M\x20\x25\x22".as_slice(),
            b"\x1b[M\x20\x25\x21".as_slice(),
        ),
    ] {
        let report = parse_report(input, 0).unwrap();
        assert_eq!(shift_row(&report, 1), Shifted::Grid(expected.to_vec()));
    }
    for input in [b"\x1b[<0;5;0M".as_slice(), b"\x1b[M\x20\x25\x21".as_slice()] {
        let report = parse_report(input, 0).unwrap();
        assert!(matches!(shift_row(&report, 1), Shifted::Claimed(_)));
        assert_eq!(shift_row(&report, 0), Shifted::Grid(input.to_vec()));
    }
    let release = parse_report(b"\x1b[<64;10;24m", 0).unwrap();
    assert_eq!(release.button, 64);
    assert!(release.release);
    let x10 = parse_report(b"\x1b[M\x23\x25\x21", 0).unwrap();
    assert!(x10.release);
}

#[test]
fn malformed_and_partial_mouse_reports_are_rejected() {
    for input in [
        b"\x1b[<0;5;2".as_slice(),
        b"\x1b[M\x20\x25".as_slice(),
        b"\x1b[M\x1f\x25\x21".as_slice(),
        b"x[<0;5;2M".as_slice(),
        b"\x1b[<99999999999999999999999;5;2M".as_slice(),
    ] {
        assert!(parse_report(input, 0).is_none());
    }
    assert!(parse_report(b"abc", usize::MAX).is_none());
    let report = parse_report(b"xx\x1b[<0;5;2M!", 2).unwrap();
    assert_eq!(report.end, 11);
    assert_eq!(
        shift_row(&report, 0),
        Shifted::Grid(b"\x1b[<0;5;2M".to_vec())
    );
}
