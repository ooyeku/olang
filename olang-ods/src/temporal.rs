//! Dates and datetimes as integers.
//!
//! A `Date` column is days since 1970-01-01 (`i32`), a `Datetime` column
//! microseconds since 1970-01-01T00:00:00 (`i64`, naive or UTC). Every
//! calendar answer is arithmetic on those integers — Howard Hinnant's
//! civil-from-days, whose divisions are by constants and compile to
//! multiplies — so a column's years, months or weekdays are one
//! branch-light pass with no per-row parsing, allocation or table.
//!
//! Text comes in through a fixed-width fast path for ISO 8601
//! (`2024-03-01`, `2024-03-01T12:30[:45[.123456]][Z|±hh:mm]`) and,
//! given a format, through a small `strptime` (`%Y %m %d %H %M %S %f %y
//! %b %B %j %p %I %z %a %A %e %F %T %%`). It goes out as ISO 8601. A
//! truncation's unit is `time::Every`, shared with `resample`.

/// Microseconds in a second, a minute, an hour, a day.
pub const US_PER_SEC: i64 = 1_000_000;
pub const US_PER_MIN: i64 = 60 * US_PER_SEC;
pub const US_PER_HOUR: i64 = 60 * US_PER_MIN;
pub const US_PER_DAY: i64 = 24 * US_PER_HOUR;

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
#[inline]
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The civil date of days since 1970-01-01: (year, month, day).
#[inline]
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The year alone (cheaper than the whole date: no day arithmetic).
#[inline]
pub fn year_of(z: i64) -> i64 {
    civil_from_days(z).0
}

pub fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

/// A real date: month 1–12, day within its month.
pub fn valid_ymd(y: i64, m: u32, d: u32) -> bool {
    (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)
}

/// ISO weekday: 1 = Monday … 7 = Sunday (1970-01-01 was a Thursday).
#[inline]
pub fn iso_weekday(z: i64) -> u32 {
    ((z + 3).rem_euclid(7) + 1) as u32
}

/// Day of the year, 1 = January 1st.
#[inline]
pub fn day_of_year(z: i64) -> u32 {
    let y = year_of(z);
    (z - days_from_civil(y, 1, 1) + 1) as u32
}

/// The ISO 8601 week number (1–53): weeks start on Monday, and week 1
/// is the one holding the year's first Thursday.
pub fn iso_week(z: i64) -> u32 {
    // The Thursday of this date's week decides its ISO year.
    let thursday = z - iso_weekday(z) as i64 + 4;
    let y = year_of(thursday);
    ((thursday - days_from_civil(y, 1, 1)) / 7 + 1) as u32
}

/// `n` months after the date, the day clamped to the month's last
/// (January 31st plus a month is February's last day). None past the
/// range of an `i32` day.
pub fn add_months(z: i64, n: i64) -> Option<i64> {
    let (y, m, d) = civil_from_days(z);
    let idx = y.checked_mul(12)?.checked_add(m as i64 - 1)?.checked_add(n)?;
    let (ny, nm) = (idx.div_euclid(12), (idx.rem_euclid(12) + 1) as u32);
    let nd = d.min(days_in_month(ny, nm));
    Some(days_from_civil(ny, nm, nd))
}

/// Whole calendar months from `b` to `a`: `(year, month)` apart, the
/// days ignored (2024-03-31 and 2024-04-01 are a month apart).
#[inline]
pub fn months_between(a: i64, b: i64) -> i64 {
    let (ya, ma, _) = civil_from_days(a);
    let (yb, mb, _) = civil_from_days(b);
    (ya - yb) * 12 + ma as i64 - mb as i64
}

pub const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// ISO order: Monday first.
pub const DAY_NAMES: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// A Datetime's day (floor division: before 1970 too).
#[inline]
pub fn day_of_us(us: i64) -> i64 {
    us.div_euclid(US_PER_DAY)
}

/// Microseconds into its day.
#[inline]
pub fn time_of_us(us: i64) -> i64 {
    us.rem_euclid(US_PER_DAY)
}

// ---------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------

/// `v` space-padded to two characters (`%e`, `%k`, `%l`).
fn push_space(out: &mut String, v: i64) {
    if (0..10).contains(&v) {
        out.push(' ');
    }
    out.push_str(&v.to_string());
}

fn push_num(out: &mut String, v: i64, width: usize) {
    if v < 0 {
        out.push('-');
    }
    let s = v.unsigned_abs().to_string();
    for _ in s.len()..width {
        out.push('0');
    }
    out.push_str(&s);
}

/// `YYYY-MM-DD`.
pub fn format_date(z: i64) -> String {
    let mut out = String::with_capacity(10);
    write_date(&mut out, z);
    out
}

fn write_date(out: &mut String, z: i64) {
    let (y, m, d) = civil_from_days(z);
    if (0..=9999).contains(&y) {
        let b = [
            b'0' + (y / 1000) as u8,
            b'0' + (y / 100 % 10) as u8,
            b'0' + (y / 10 % 10) as u8,
            b'0' + (y % 10) as u8,
            b'-',
            b'0' + (m / 10) as u8,
            b'0' + (m % 10) as u8,
            b'-',
            b'0' + (d / 10) as u8,
            b'0' + (d % 10) as u8,
        ];
        out.push_str(std::str::from_utf8(&b).expect("ascii"));
    } else {
        push_num(out, y, 4);
        out.push('-');
        push_num(out, m as i64, 2);
        out.push('-');
        push_num(out, d as i64, 2);
    }
}

/// `YYYY-MM-DDTHH:MM:SS`, with `.ffffff` (trailing zeros dropped) when
/// the second has a fraction.
pub fn format_datetime(us: i64) -> String {
    let mut out = String::with_capacity(26);
    write_date(&mut out, day_of_us(us));
    let t = time_of_us(us);
    out.push('T');
    push_num(&mut out, t / US_PER_HOUR, 2);
    out.push(':');
    push_num(&mut out, t / US_PER_MIN % 60, 2);
    out.push(':');
    push_num(&mut out, t / US_PER_SEC % 60, 2);
    let frac = t % US_PER_SEC;
    if frac != 0 {
        let mut f = format!("{:06}", frac);
        while f.ends_with('0') {
            f.pop();
        }
        out.push('.');
        out.push_str(&f);
    }
    out
}

// ---------------------------------------------------------------------
// Reading ISO 8601
// ---------------------------------------------------------------------

#[inline]
fn digits(b: &[u8], at: usize, n: usize) -> Option<u32> {
    let s = b.get(at..at + n)?;
    let mut v = 0u32;
    for &c in s {
        let d = c.wrapping_sub(b'0');
        if d > 9 {
            return None;
        }
        v = v * 10 + d as u32;
    }
    Some(v)
}

/// `YYYY-MM-DD` exactly: the fixed-width fast path.
#[inline]
pub fn parse_iso_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y = digits(b, 0, 4)? as i64;
    let m = digits(b, 5, 2)?;
    let d = digits(b, 8, 2)?;
    if !valid_ymd(y, m, d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// An ISO 8601 date or datetime as microseconds, and whether it had a
/// time: `YYYY-MM-DD`, then optionally `T` or a space, `HH:MM`,
/// `:SS`, `.f…` (up to microseconds; more digits are cut), and `Z` or
/// an offset `±HH[:MM]` (the moment converted to UTC).
pub fn parse_iso(s: &str) -> Option<(i64, bool)> {
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y = digits(b, 0, 4)? as i64;
    let m = digits(b, 5, 2)?;
    let d = digits(b, 8, 2)?;
    if !valid_ymd(y, m, d) {
        return None;
    }
    let day = days_from_civil(y, m, d);
    if b.len() == 10 {
        return Some((day * US_PER_DAY, false));
    }
    if !(b[10] == b'T' || b[10] == b' ') || b.len() < 16 || b[13] != b':' {
        return None;
    }
    let hh = digits(b, 11, 2)? as i64;
    let mm = digits(b, 14, 2)? as i64;
    let mut at = 16;
    let mut ss = 0i64;
    let mut frac = 0i64;
    if b.get(at) == Some(&b':') {
        ss = digits(b, at + 1, 2)? as i64;
        at += 3;
        if b.get(at) == Some(&b'.') || b.get(at) == Some(&b',') {
            at += 1;
            let start = at;
            let mut scale = 100_000i64;
            while let Some(&c) = b.get(at) {
                if !c.is_ascii_digit() {
                    break;
                }
                frac += (c - b'0') as i64 * scale;
                scale /= 10;
                at += 1;
            }
            if at == start {
                return None;
            }
        }
    }
    if hh > 24 || mm > 59 || ss > 60 || (hh == 24 && (mm, ss, frac) != (0, 0, 0)) {
        return None;
    }
    let mut us = day * US_PER_DAY + hh * US_PER_HOUR + mm * US_PER_MIN + ss * US_PER_SEC + frac;
    let rest = &b[at..];
    match rest {
        [] | [b'Z'] => {}
        [sign @ (b'+' | b'-'), tail @ ..] => {
            let (oh, om) = match tail.len() {
                2 => (digits(tail, 0, 2)?, 0),
                4 => (digits(tail, 0, 2)?, digits(tail, 2, 2)?),
                5 if tail[2] == b':' => (digits(tail, 0, 2)?, digits(tail, 3, 2)?),
                _ => return None,
            };
            let off = oh as i64 * US_PER_HOUR + om as i64 * US_PER_MIN;
            us -= if *sign == b'+' { off } else { -off };
        }
        _ => return None,
    }
    Some((us, true))
}

// ---------------------------------------------------------------------
// Formats
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Item {
    Lit(String),
    /// One or more spaces in the format: any run of whitespace.
    Space,
    Year,
    Year2,
    /// `%C`: the century, two digits.
    Century,
    Month,
    MonthName,
    MonthAbbr,
    Day,
    /// `%e`: the day, space-padded to two.
    DaySpace,
    DayOfYear,
    Hour,
    /// `%k`: the hour, space-padded.
    HourSpace,
    Hour12,
    /// `%l`: the 12-hour hour, space-padded.
    Hour12Space,
    AmPm,
    /// `%P`: am or pm.
    AmPmLower,
    Minute,
    Second,
    Frac,
    Offset,
    /// `%Z`: the zone's name (UTC: a time is naive or UTC).
    Zone,
    WeekdayName,
    WeekdayAbbr,
    /// `%u`: ISO, 1 for Monday to 7 for Sunday.
    WeekdayIso,
    /// `%w`: 0 for Sunday to 6 for Saturday.
    WeekdaySun,
    /// `%U`: the week of the year, Sunday first, 00–53.
    WeekSun,
    /// `%W`: the week of the year, Monday first, 00–53.
    WeekMon,
    /// `%V`: the ISO 8601 week, 01–53.
    IsoWeek,
    /// `%G`: the ISO 8601 week's year.
    IsoYear,
    /// `%s`: seconds since the epoch.
    Epoch,
}

/// The directives a format knows, for its error.
const DIRECTIVES: &str = "%Y %y %C %G %m %b %B %h %d %e %j %U %W %V %u %w %a %A %H %k %I %l %p %P %M %S %f %z %Z %s %F %T %D %R %c %x %X %n %t %%";

/// A parsed `strptime`/`strftime` format.
#[derive(Clone, Debug)]
pub struct Format {
    items: Vec<Item>,
    /// Whether it reads a time of day (an hour, minute, second or
    /// fraction).
    pub has_time: bool,
}

impl Format {
    /// Parse a format, or say which directive it does not know.
    pub fn new(fmt: &str) -> Result<Format, String> {
        let mut items = Vec::new();
        let mut lit = String::new();
        let mut chars = fmt.chars().peekable();
        let flush = |lit: &mut String, items: &mut Vec<Item>| {
            if !lit.is_empty() {
                items.push(Item::Lit(std::mem::take(lit)));
            }
        };
        let lit_item = |t: &str| Item::Lit(t.to_string());
        while let Some(c) = chars.next() {
            if c == ' ' {
                flush(&mut lit, &mut items);
                while chars.peek() == Some(&' ') {
                    chars.next();
                }
                items.push(Item::Space);
                continue;
            }
            if c != '%' {
                lit.push(c);
                continue;
            }
            let Some(d) = chars.next() else {
                return Err("the format ends in a lone %".to_string());
            };
            match d {
                '%' => {
                    lit.push('%');
                    continue;
                }
                'n' => {
                    lit.push('\n');
                    continue;
                }
                't' => {
                    lit.push('\t');
                    continue;
                }
                _ => {}
            }
            flush(&mut lit, &mut items);
            match d {
                'Y' => items.push(Item::Year),
                'y' => items.push(Item::Year2),
                'C' => items.push(Item::Century),
                'G' => items.push(Item::IsoYear),
                'm' => items.push(Item::Month),
                'B' => items.push(Item::MonthName),
                'b' | 'h' => items.push(Item::MonthAbbr),
                'd' => items.push(Item::Day),
                'e' => items.push(Item::DaySpace),
                'j' => items.push(Item::DayOfYear),
                'U' => items.push(Item::WeekSun),
                'W' => items.push(Item::WeekMon),
                'V' => items.push(Item::IsoWeek),
                'u' => items.push(Item::WeekdayIso),
                'w' => items.push(Item::WeekdaySun),
                'H' => items.push(Item::Hour),
                'k' => items.push(Item::HourSpace),
                'I' => items.push(Item::Hour12),
                'l' => items.push(Item::Hour12Space),
                'p' => items.push(Item::AmPm),
                'P' => items.push(Item::AmPmLower),
                'M' => items.push(Item::Minute),
                'S' => items.push(Item::Second),
                'f' => items.push(Item::Frac),
                'z' => items.push(Item::Offset),
                'Z' => items.push(Item::Zone),
                's' => items.push(Item::Epoch),
                'A' => items.push(Item::WeekdayName),
                'a' => items.push(Item::WeekdayAbbr),
                'F' => items.extend([
                    Item::Year,
                    lit_item("-"),
                    Item::Month,
                    lit_item("-"),
                    Item::Day,
                ]),
                'T' | 'X' => items.extend([
                    Item::Hour,
                    lit_item(":"),
                    Item::Minute,
                    lit_item(":"),
                    Item::Second,
                ]),
                'D' | 'x' => items.extend([
                    Item::Month,
                    lit_item("/"),
                    Item::Day,
                    lit_item("/"),
                    Item::Year2,
                ]),
                'R' => items.extend([Item::Hour, lit_item(":"), Item::Minute]),
                // The C locale's: `Mon Jan  1 00:00:00 2024`.
                'c' => items.extend([
                    Item::WeekdayAbbr,
                    Item::Space,
                    Item::MonthAbbr,
                    Item::Space,
                    Item::DaySpace,
                    Item::Space,
                    Item::Hour,
                    lit_item(":"),
                    Item::Minute,
                    lit_item(":"),
                    Item::Second,
                    Item::Space,
                    Item::Year,
                ]),
                other => {
                    return Err(format!(
                        "the format has %{}, which is not one of {}",
                        other, DIRECTIVES
                    ));
                }
            }
        }
        flush(&mut lit, &mut items);
        let has_time = items.iter().any(|i| {
            matches!(
                i,
                Item::Hour
                    | Item::HourSpace
                    | Item::Hour12
                    | Item::Hour12Space
                    | Item::Minute
                    | Item::Second
                    | Item::Frac
                    | Item::Epoch
            )
        });
        Ok(Format { items, has_time })
    }

    /// A format to read with: one with a directive that is only written
    /// (a week number, a century, an ISO year) is refused.
    pub fn reader(fmt: &str) -> Result<Format, String> {
        let f = Format::new(fmt)?;
        if f.items.iter().any(|i| {
            matches!(
                i,
                Item::Century | Item::IsoYear | Item::WeekSun | Item::WeekMon | Item::IsoWeek
            )
        }) {
            return Err(format!(
                "\"{}\" has %C, %G, %U, %V or %W, which format a date but say too little to read one",
                fmt
            ));
        }
        Ok(f)
    }

    /// Read `s` as microseconds since the epoch (a date alone is its
    /// midnight). None when it does not match or is no real date.
    pub fn parse(&self, s: &str) -> Option<i64> {
        let b = s.as_bytes();
        let mut at = 0usize;
        let (mut y, mut m, mut d) = (1970i64, 1u32, 1u32);
        let mut doy: Option<u32> = None;
        let (mut hh, mut mi, mut ss, mut frac) = (0i64, 0i64, 0i64, 0i64);
        let mut pm: Option<bool> = None;
        let mut off = 0i64;
        let mut epoch: Option<i64> = None;
        // Up to `max` digits (at least one).
        let num = |at: &mut usize, max: usize| -> Option<i64> {
            let start = *at;
            let mut v = 0i64;
            while *at < b.len() && *at - start < max && b[*at].is_ascii_digit() {
                v = v * 10 + (b[*at] - b'0') as i64;
                *at += 1;
            }
            (*at > start).then_some(v)
        };
        let name = |at: &mut usize, names: &[&str]| -> Option<usize> {
            let rest = &s[*at..];
            for (i, n) in names.iter().enumerate() {
                for cand in [*n, &n[..3]] {
                    if rest.len() >= cand.len()
                        && rest.as_bytes()[..cand.len()].eq_ignore_ascii_case(cand.as_bytes())
                    {
                        *at += cand.len();
                        return Some(i);
                    }
                }
            }
            None
        };
        for item in &self.items {
            match item {
                Item::Lit(t) => {
                    if !s[at..].starts_with(t.as_str()) {
                        return None;
                    }
                    at += t.len();
                }
                Item::Space => {
                    let start = at;
                    while at < b.len() && b[at].is_ascii_whitespace() {
                        at += 1;
                    }
                    if at == start {
                        return None;
                    }
                }
                Item::Year => {
                    let neg = b.get(at) == Some(&b'-');
                    if neg {
                        at += 1;
                    }
                    let v = num(&mut at, 4)?;
                    y = if neg { -v } else { v };
                }
                Item::Year2 => {
                    let v = num(&mut at, 2)?;
                    // POSIX: 69–99 are 1969–1999, 00–68 are 2000–2068.
                    y = if v >= 69 { 1900 + v } else { 2000 + v };
                }
                Item::Month => m = num(&mut at, 2)? as u32,
                Item::MonthName | Item::MonthAbbr => {
                    m = name(&mut at, &MONTH_NAMES)? as u32 + 1
                }
                Item::Day | Item::DaySpace => {
                    if b.get(at) == Some(&b' ') {
                        at += 1;
                    }
                    d = num(&mut at, 2)? as u32
                }
                Item::DayOfYear => doy = Some(num(&mut at, 3)? as u32),
                Item::Hour | Item::Hour12 => hh = num(&mut at, 2)?,
                Item::HourSpace | Item::Hour12Space => {
                    if b.get(at) == Some(&b' ') {
                        at += 1;
                    }
                    hh = num(&mut at, 2)?
                }
                Item::AmPm | Item::AmPmLower => {
                    let rest = &s[at..];
                    if rest.len() < 2 {
                        return None;
                    }
                    pm = Some(match &rest[..2] {
                        x if x.eq_ignore_ascii_case("am") => false,
                        x if x.eq_ignore_ascii_case("pm") => true,
                        _ => return None,
                    });
                    at += 2;
                }
                Item::Minute => mi = num(&mut at, 2)?,
                Item::Second => ss = num(&mut at, 2)?,
                Item::Frac => {
                    let start = at;
                    let mut scale = 100_000i64;
                    while at < b.len() && b[at].is_ascii_digit() {
                        frac += (b[at] - b'0') as i64 * scale;
                        scale /= 10;
                        at += 1;
                    }
                    if at == start {
                        return None;
                    }
                }
                Item::Offset => {
                    if b.get(at) == Some(&b'Z') {
                        at += 1;
                        continue;
                    }
                    let sign = match b.get(at) {
                        Some(b'+') => 1,
                        Some(b'-') => -1,
                        _ => return None,
                    };
                    at += 1;
                    let oh = num(&mut at, 2)?;
                    if b.get(at) == Some(&b':') {
                        at += 1;
                    }
                    let om = num(&mut at, 2).unwrap_or(0);
                    off = sign * (oh * US_PER_HOUR + om * US_PER_MIN);
                }
                Item::WeekdayName | Item::WeekdayAbbr => {
                    name(&mut at, &DAY_NAMES)?;
                }
                // A weekday's number, like its name, is read and not
                // checked.
                Item::WeekdayIso | Item::WeekdaySun => {
                    num(&mut at, 1)?;
                }
                Item::Zone => {
                    let rest = &s[at..];
                    let n = ["UTC", "GMT", "Z"]
                        .iter()
                        .find(|z| rest.starts_with(**z))?
                        .len();
                    at += n;
                }
                Item::Epoch => {
                    let neg = b.get(at) == Some(&b'-');
                    if neg {
                        at += 1;
                    }
                    let v = num(&mut at, 18)?;
                    epoch = Some(if neg { -v } else { v });
                }
                // Only written: a week number or a century says too
                // little to read a date back.
                Item::Century | Item::IsoYear | Item::WeekSun | Item::WeekMon | Item::IsoWeek => {
                    return None;
                }
            }
        }
        if at != b.len() {
            return None;
        }
        if let Some(v) = epoch {
            return v.checked_mul(US_PER_SEC)?.checked_add(frac);
        }
        if let Some(p) = pm {
            if !(1..=12).contains(&hh) {
                return None;
            }
            hh = hh % 12 + if p { 12 } else { 0 };
        }
        let day = match doy {
            Some(j) => {
                let len = if is_leap(y) { 366 } else { 365 };
                if j == 0 || j > len {
                    return None;
                }
                days_from_civil(y, 1, 1) + j as i64 - 1
            }
            None => {
                if !valid_ymd(y, m, d) {
                    return None;
                }
                days_from_civil(y, m, d)
            }
        };
        if hh > 23 || mi > 59 || ss > 60 {
            return None;
        }
        Some(day * US_PER_DAY + hh * US_PER_HOUR + mi * US_PER_MIN + ss * US_PER_SEC + frac - off)
    }

    /// Write microseconds since the epoch in this format.
    pub fn format(&self, us: i64) -> String {
        let day = day_of_us(us);
        let t = time_of_us(us);
        let (y, m, d) = civil_from_days(day);
        let mut out = String::new();
        for item in &self.items {
            match item {
                Item::Lit(s) => out.push_str(s),
                Item::Space => out.push(' '),
                Item::Year => push_num(&mut out, y, 4),
                Item::Year2 => push_num(&mut out, y.rem_euclid(100), 2),
                Item::Century => push_num(&mut out, y.div_euclid(100), 2),
                Item::Month => push_num(&mut out, m as i64, 2),
                Item::MonthName => out.push_str(MONTH_NAMES[m as usize - 1]),
                Item::MonthAbbr => out.push_str(&MONTH_NAMES[m as usize - 1][..3]),
                Item::Day => push_num(&mut out, d as i64, 2),
                Item::DaySpace => push_space(&mut out, d as i64),
                Item::DayOfYear => push_num(&mut out, day_of_year(day) as i64, 3),
                Item::Hour => push_num(&mut out, t / US_PER_HOUR, 2),
                Item::HourSpace => push_space(&mut out, t / US_PER_HOUR),
                Item::Hour12 | Item::Hour12Space => {
                    let h = t / US_PER_HOUR % 12;
                    let h = if h == 0 { 12 } else { h };
                    if *item == Item::Hour12 {
                        push_num(&mut out, h, 2)
                    } else {
                        push_space(&mut out, h)
                    }
                }
                Item::AmPm => out.push_str(if t / US_PER_HOUR >= 12 { "PM" } else { "AM" }),
                Item::AmPmLower => out.push_str(if t / US_PER_HOUR >= 12 { "pm" } else { "am" }),
                Item::Minute => push_num(&mut out, t / US_PER_MIN % 60, 2),
                Item::Second => push_num(&mut out, t / US_PER_SEC % 60, 2),
                Item::Frac => out.push_str(&format!("{:06}", t % US_PER_SEC)),
                Item::Offset => out.push_str("+0000"),
                Item::Zone => out.push_str("UTC"),
                Item::Epoch => out.push_str(&us.div_euclid(US_PER_SEC).to_string()),
                Item::WeekdayName => out.push_str(DAY_NAMES[iso_weekday(day) as usize - 1]),
                Item::WeekdayAbbr => out.push_str(&DAY_NAMES[iso_weekday(day) as usize - 1][..3]),
                Item::WeekdayIso => push_num(&mut out, iso_weekday(day) as i64, 1),
                Item::WeekdaySun => push_num(&mut out, iso_weekday(day) as i64 % 7, 1),
                Item::WeekSun | Item::WeekMon => {
                    // Days before the year's first Sunday (Monday) are
                    // week 0.
                    let doy = day_of_year(day) as i64 - 1;
                    let wd = if *item == Item::WeekSun {
                        iso_weekday(day) as i64 % 7
                    } else {
                        iso_weekday(day) as i64 - 1
                    };
                    push_num(&mut out, (doy + 7 - wd) / 7, 2)
                }
                Item::IsoWeek => push_num(&mut out, iso_week(day) as i64, 2),
                Item::IsoYear => push_num(&mut out, year_of(day - iso_weekday(day) as i64 + 4), 4),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trips_every_day_of_four_centuries() {
        for z in -146097 * 2..146097 * 2 {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
            assert!(valid_ymd(y, m, d));
        }
    }

    #[test]
    fn weekdays_weeks_and_days_of_year() {
        assert_eq!(iso_weekday(0), 4); // 1970-01-01, a Thursday
        assert_eq!(iso_weekday(days_from_civil(2024, 3, 3)), 7);
        assert_eq!(iso_week(days_from_civil(2021, 1, 3)), 53);
        assert_eq!(iso_week(days_from_civil(2021, 1, 4)), 1);
        assert_eq!(iso_week(days_from_civil(2024, 12, 30)), 1);
        assert_eq!(day_of_year(days_from_civil(2024, 12, 31)), 366);
        assert_eq!(add_months(days_from_civil(2024, 1, 31), 1), Some(days_from_civil(2024, 2, 29)));
        assert_eq!(months_between(days_from_civil(2024, 4, 1), days_from_civil(2024, 3, 31)), 1);
    }

    #[test]
    fn iso_text_reads_and_writes() {
        assert_eq!(parse_iso_date("2024-03-01"), Some(19783));
        assert_eq!(parse_iso_date("2024-02-30"), None);
        assert_eq!(parse_iso("1970-01-01T01:30"), Some((90 * US_PER_MIN, true)));
        assert_eq!(parse_iso("1970-01-01 00:00:02.5Z"), Some((2_500_000, true)));
        assert_eq!(parse_iso("1970-01-01T02:00:00+02:00"), Some((0, true)));
        assert_eq!(format_datetime(2_500_000), "1970-01-01T00:00:02.5");
        assert_eq!(format_date(-1), "1969-12-31");
    }

    #[test]
    fn formats_read_and_write() {
        let f = Format::new("%d/%m/%Y %H:%M").unwrap();
        assert_eq!(f.parse("02/01/1970 00:30"), Some(US_PER_DAY + 30 * US_PER_MIN));
        assert_eq!(f.format(US_PER_DAY + 30 * US_PER_MIN), "02/01/1970 00:30");
        let g = Format::new("%b %d, %Y %I:%M %p").unwrap();
        assert_eq!(g.parse("Jan 01, 1970 12:15 PM"), Some(12 * US_PER_HOUR + 15 * US_PER_MIN));
        assert!(Format::new("%Q").is_err());
        // Every directive against Python's strftime (C locale) for
        // 2024-01-07T15:04:05.25, a Sunday.
        let t = days_from_civil(2024, 1, 7) * US_PER_DAY
            + 15 * US_PER_HOUR
            + 4 * US_PER_MIN
            + 5 * US_PER_SEC
            + 250_000;
        for (fmt, want) in [
            ("%Y %y %C %G", "2024 24 20 2024"),
            ("%m %b %B %h", "01 Jan January Jan"),
            ("%d|%e|%j", "07| 7|007"),
            ("%U %W %V %u %w", "01 01 01 7 0"),
            ("%a %A", "Sun Sunday"),
            ("%H %k %I %l %p %P", "15 15 03  3 PM pm"),
            ("%M %S %f", "04 05 250000"),
            ("%z %Z %s", "+0000 UTC 1704639845"),
            ("%F %T", "2024-01-07 15:04:05"),
            ("%D %R", "01/07/24 15:04"),
            ("%c", "Sun Jan  7 15:04:05 2024"),
            ("%x %X", "01/07/24 15:04:05"),
            ("%%%n%t", "%\n\t"),
        ] {
            assert_eq!(Format::new(fmt).unwrap().format(t), want, "{}", fmt);
        }
        // The ISO year of a late-December Monday is the next.
        let d = days_from_civil(2024, 12, 30) * US_PER_DAY;
        assert_eq!(Format::new("%G-W%V-%u").unwrap().format(d), "2025-W01-1");
        // Week 0 holds the days before the year's first Sunday / Monday.
        let jan1 = days_from_civil(2023, 1, 1) * US_PER_DAY; // a Sunday
        assert_eq!(Format::new("%U %W").unwrap().format(jan1), "01 00");
        assert_eq!(Format::new("%b %e %Y").unwrap().parse("Jan  7 2024"), Some(days_from_civil(2024, 1, 7) * US_PER_DAY));
        assert_eq!(Format::new("%s").unwrap().parse("86400"), Some(US_PER_DAY));
        assert_eq!(Format::new("%Y-%V").unwrap().parse("2024-01"), None);
        assert_eq!(Format::new("%Y-%j").unwrap().parse("1970-032"), Some(31 * US_PER_DAY));
    }
}
