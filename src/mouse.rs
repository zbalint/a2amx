//! Parsing and translating terminal mouse reports for reserved top rows.

use crate::messaging::sgr_mouse_report_len;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Sgr,
    X10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseReport<'a> {
    pub encoding: Encoding,
    pub button: u16,
    pub release: bool,
    pub column: u16,
    pub row: u16,
    pub end: usize,
    bytes: &'a [u8],
    row_start: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shifted<'a> {
    Claimed(MouseReport<'a>),
    Grid(Vec<u8>),
}

pub fn parse_report(bytes: &[u8], start: usize) -> Option<MouseReport<'_>> {
    let input = bytes.get(start..)?;
    if input.starts_with(b"\x1b[<") {
        let end = sgr_mouse_report_len(bytes, start)?;
        let report = bytes.get(start..end)?;
        let first = report.iter().position(|&b| b == b';')?;
        let second = report.get(first + 1..)?.iter().position(|&b| b == b';')? + first + 1;
        let number = |value: &[u8]| std::str::from_utf8(value).ok()?.parse::<u16>().ok();
        Some(MouseReport {
            encoding: Encoding::Sgr,
            button: number(&report[3..first])?,
            release: *report.last()? == b'm',
            column: number(&report[first + 1..second])?,
            row: number(&report[second + 1..report.len() - 1])?,
            end,
            bytes: report,
            row_start: second + 1,
        })
    } else if input.starts_with(b"\x1b[M") {
        let report = input.get(..6)?;
        let button = report[3].checked_sub(32)?;
        let column = report[4].checked_sub(32)?;
        let row = report[5].checked_sub(32)?;
        Some(MouseReport {
            encoding: Encoding::X10,
            button: u16::from(button),
            release: button == 3,
            column: u16::from(column),
            row: u16::from(row),
            end: start.checked_add(6)?,
            bytes: report,
            row_start: 5,
        })
    } else {
        None
    }
}

pub fn shift_row<'a>(report: &MouseReport<'a>, origin: u16) -> Shifted<'a> {
    if origin > 0 && report.row <= origin {
        return Shifted::Claimed(*report);
    }
    if origin == 0 {
        return Shifted::Grid(report.bytes.to_vec());
    }
    Shifted::Grid(with_row(report, report.row - origin))
}

pub fn with_row(report: &MouseReport<'_>, row: u16) -> Vec<u8> {
    let mut bytes = report.bytes[..report.row_start].to_vec();
    match report.encoding {
        Encoding::X10 => bytes.push(u8::try_from(row.saturating_add(32)).unwrap_or(u8::MAX)),
        Encoding::Sgr => {
            bytes.extend_from_slice(row.to_string().as_bytes());
            bytes.push(if report.release { b'm' } else { b'M' });
        }
    }
    bytes
}
