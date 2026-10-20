//! Time buckets: an interval (`"1d"`, `"15m"`, `"1mo"`) and the floor of
//! a time column to it — what `resample` groups on.
//!
//! A time column today is ISO text (`2024-03-01`, `2024-03-01T13:45`,
//! `2024-03-01 13:45:10`): the floor parses each value (a fixed-width
//! fast path, civil-from-days arithmetic, no allocation but the output)
//! and writes the bucket's start in the input's own shape — a date for a
//! bucket of a day or more, the date and time (with the input's
//! separator, and seconds when it had them) for a shorter one. The
//! arithmetic works on days and seconds, so a `Date`/`Datetime` column
//! floors with the same `Every` on its integer storage.

use crate::{OdsError, Series};

type Result<T> = std::result::Result<T, OdsError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Second,
    Minute,
    Hour,
    Day,
    /// Weeks start on Monday (ISO).
    Week,
    Month,
    Year,
}

/// A bucket width: `n` units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Every {
    pub n: i64,
    pub unit: Unit,
}

impl Every {
    /// `"1d"`, `"6h"`, `"15m"` (or `"15min"`), `"30s"`, `"1w"`, `"1mo"`,
    /// `"1q"` (three months), `"1y"`; a bare unit (`"d"`, `"day"`,
    /// `"month"`, …) is one of it.
    pub fn parse(text: &str) -> Result<Every> {
        let t = text.trim();
        let split = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
        let (num, unit) = t.split_at(split);
        let n: i64 = if num.is_empty() {
            1
        } else {
            num.parse().map_err(|_| bad_every(text))?
        };
        if n < 1 {
            return Err(bad_every(text));
        }
        let (n, unit) = match unit {
            "s" | "sec" | "second" | "seconds" => (n, Unit::Second),
            "m" | "min" | "minute" | "minutes" => (n, Unit::Minute),
            "h" | "hour" | "hours" => (n, Unit::Hour),
            "d" | "day" | "days" => (n, Unit::Day),
            "w" | "week" | "weeks" => (n, Unit::Week),
            "mo" | "month" | "months" => (n, Unit::Month),
            "q" | "quarter" | "quarters" => (n * 3, Unit::Month),
            "y" | "year" | "years" => (n, Unit::Year),
            _ => return Err(bad_every(text)),
        };
        Ok(Every { n, unit })
    }

    /// Whether the bucket is a day or longer (its start is a date).
    pub fn is_dated(self) -> bool {
        matches!(self.unit, Unit::Day | Unit::Week | Unit::Month | Unit::Year)
    }

    /// The start of the bucket holding the instant `days` (since
    /// 1970-01-01) plus `secs` into that day: as (days, secs).
    pub fn floor(self, days: i64, secs: i64) -> (i64, i64) {
        let n = self.n;
        match self.unit {
            Unit::Second | Unit::Minute | Unit::Hour => {
                let step = n * match self.unit {
                    Unit::Second => 1,
                    Unit::Minute => 60,
                    _ => 3600,
                };
                let t = days * 86_400 + secs;
                let f = t.div_euclid(step) * step;
                (f.div_euclid(86_400), f.rem_euclid(86_400))
            }
            Unit::Day => (days.div_euclid(n) * n, 0),
            // 1970-01-05 was a Monday.
            Unit::Week => ((days - 4).div_euclid(7 * n) * 7 * n + 4, 0),
            Unit::Month => {
                let (y, m, _) = civil_from_days(days);
                let months = (y * 12 + (m - 1)).div_euclid(n) * n;
                (days_from_civil(months.div_euclid(12), months.rem_euclid(12) + 1, 1), 0)
            }
            Unit::Year => {
                let (y, _, _) = civil_from_days(days);
                (days_from_civil(y.div_euclid(n) * n, 1, 1), 0)
            }
        }
    }
}

fn bad_every(text: &str) -> OdsError {
    OdsError::InvalidArgument(format!(
        "'{}' is not an interval: write a count and a unit, like \"1d\", \"6h\", \"15m\", \
         \"30s\", \"1w\", \"1mo\", \"1q\" or \"1y\"",
        text
    ))
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// How an ISO value was written, so a floor writes its bucket the same
/// way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// The date-time separator (`T` or a space); None for a bare date.
    sep: Option<u8>,
    seconds: bool,
}

fn digits(b: &[u8]) -> Option<i64> {
    let mut v = 0i64;
    for &c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as i64;
    }
    Some(v)
}

/// An ISO date or date-time: (days, seconds into the day, shape). A
/// fraction of a second and a trailing `Z` are read and dropped.
fn parse_iso(text: &str) -> Option<(i64, i64, Shape)> {
    let b = text.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, m, d) = (digits(&b[0..4])?, digits(&b[5..7])?, digits(&b[8..10])?);
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    let days = days_from_civil(y, m, d);
    let rest = &b[10..];
    if rest.is_empty() {
        return Some((days, 0, Shape { sep: None, seconds: false }));
    }
    let sep = rest[0];
    if !(sep == b'T' || sep == b' ') || rest.len() < 6 || rest[3] != b':' {
        return None;
    }
    let (hh, mm) = (digits(&rest[1..3])?, digits(&rest[4..6])?);
    let mut tail = &rest[6..];
    let mut ss = 0;
    let mut seconds = false;
    if tail.first() == Some(&b':') {
        ss = digits(tail.get(1..3)?)?;
        seconds = true;
        tail = &tail[3..];
        if tail.first() == Some(&b'.') {
            let k = tail[1..].iter().take_while(|c| c.is_ascii_digit()).count();
            tail = &tail[1 + k..];
        }
    }
    if tail == b"Z" {
        tail = &[];
    }
    if !tail.is_empty() || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    Some((days, hh * 3600 + mm * 60 + ss, Shape { sep: Some(sep), seconds }))
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn write_iso(out: &mut String, days: i64, secs: i64, every: Every, shape: Shape) {
    use std::fmt::Write;
    let (y, m, d) = civil_from_days(days);
    let _ = write!(out, "{:04}-{:02}-{:02}", y, m, d);
    if every.is_dated() {
        return;
    }
    let sep = shape.sep.unwrap_or(b'T') as char;
    let _ = write!(out, "{}{:02}:{:02}", sep, secs / 3600, secs % 3600 / 60);
    if shape.seconds || every.unit == Unit::Second {
        let _ = write!(out, ":{:02}", secs % 60);
    }
}

/// Each value's bucket of width `every`, as its start in seconds since
/// 1970-01-01T00:00 (an Int column, nulls kept: what a grouping keys on,
/// with no text per row), and the shape the text was written in.
pub fn buckets(s: &Series, every: Every) -> Result<(Series, Shape)> {
    match s {
        Series::Str { col, validity } => {
            let mut out: Vec<i64> = Vec::with_capacity(col.len());
            let mut shape: Option<Shape> = None;
            for (i, text) in col.iter().enumerate() {
                if !validity.as_ref().is_none_or(|v| v.get(i)) {
                    out.push(0);
                    continue;
                }
                let (days, secs, sh) = parse_iso(text).ok_or_else(|| {
                    OdsError::InvalidArgument(format!(
                        "{:?} (row {}) is not an ISO date or date-time like 2024-03-01 or \
                         2024-03-01T13:45",
                        text, i
                    ))
                })?;
                shape = Some(match shape {
                    None => sh,
                    Some(prev) => Shape {
                        sep: prev.sep.or(sh.sep),
                        seconds: prev.seconds || sh.seconds,
                    },
                });
                let (fd, fs) = every.floor(days, secs);
                out.push(fd * 86_400 + fs);
            }
            Ok((
                Series::I64 {
                    values: std::sync::Arc::new(out),
                    validity: validity.clone(),
                },
                shape.unwrap_or(Shape { sep: None, seconds: false }),
            ))
        }
        other => Err(OdsError::TypeMismatch(format!(
            "a time bucket needs ISO date or date-time text, got a {} column",
            other.dtype()
        ))),
    }
}

/// Bucket starts (seconds since the epoch, from `buckets`) as ISO text
/// in `shape`.
pub fn format_buckets(codes: &Series, every: Every, shape: Shape) -> Series {
    let Series::I64 { values, validity } = codes else {
        return codes.clone();
    };
    let out: Vec<Option<String>> = values
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            validity.as_ref().is_none_or(|v| v.get(i)).then(|| {
                let mut o = String::with_capacity(19);
                write_iso(&mut o, t.div_euclid(86_400), t.rem_euclid(86_400), every, shape);
                o
            })
        })
        .collect();
    Series::from_str_options(out)
}

/// The start of each value's bucket of width `every`. A String column
/// of ISO dates or date-times gives ISO text in the same shape (nulls
/// stay null); anything else raises.
pub fn floor(s: &Series, every: Every) -> Result<Series> {
    let (codes, shape) = buckets(s, every)?;
    Ok(format_buckets(&codes, every, shape))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fl(text: &str, every: &str) -> String {
        let s = Series::from_str_values(vec![text.to_string()]);
        match floor(&s, Every::parse(every).unwrap()).unwrap().scalar_at(0) {
            crate::Scalar::Str(t) => t,
            _ => panic!(),
        }
    }

    #[test]
    fn floors_in_the_inputs_shape() {
        assert_eq!(fl("2024-03-17T13:45", "1d"), "2024-03-17");
        assert_eq!(fl("2024-03-17T13:45", "1h"), "2024-03-17T13:00");
        assert_eq!(fl("2024-03-17 13:45:10", "15m"), "2024-03-17 13:45:00");
        assert_eq!(fl("2024-03-17", "1mo"), "2024-03-01");
        assert_eq!(fl("2024-08-17", "1q"), "2024-07-01");
        assert_eq!(fl("2024-03-17", "1w"), "2024-03-11");
        assert_eq!(fl("2024-03-17", "1y"), "2024-01-01");
        assert_eq!(fl("1969-12-31T23:59", "1h"), "1969-12-31T23:00");
        assert!(Every::parse("1x").is_err());
        for d in [-800_000i64, -1, 0, 59, 60, 19_000, 2_000_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
    }
}
