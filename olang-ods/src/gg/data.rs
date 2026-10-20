//! A plot's data: the frame handed in (named columns), and the layer
//! frames the pipeline computes (one column per aesthetic).

use super::Aes;
use std::collections::BTreeMap;
use std::sync::Arc;

/// A column handed in: numbers (NaN for null) or text (None for null).
#[derive(Clone, Debug, PartialEq)]
pub enum Column {
    Num(Vec<f64>),
    Str(Vec<Option<String>>),
}

impl Column {
    pub fn len(&self) -> usize {
        match self {
            Column::Num(v) => v.len(),
            Column::Str(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A frame handed in: named columns of one length.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub n: usize,
    pub names: Vec<String>,
    pub cols: Vec<Column>,
}

impl Data {
    pub fn new(names: Vec<String>, cols: Vec<Column>) -> Data {
        let n = cols.first().map(|c| c.len()).unwrap_or(0);
        Data { n, names, cols }
    }

    pub fn get(&self, name: &str) -> Option<&Column> {
        self.names
            .iter()
            .position(|n| n == name)
            .map(|i| &self.cols[i])
    }
}

/// NA in a categorical column.
pub const NA: u32 = u32::MAX;

/// A layer frame's column: numbers or codes into levels.
#[derive(Clone, Debug)]
pub enum Col {
    Num(Vec<f64>),
    Cat(Vec<u32>, Arc<Vec<String>>),
}

impl Col {
    pub fn take(&self, idx: &[usize]) -> Col {
        match self {
            Col::Num(v) => Col::Num(idx.iter().map(|&i| v[i]).collect()),
            Col::Cat(c, l) => Col::Cat(idx.iter().map(|&i| c[i]).collect(), l.clone()),
        }
    }

    pub fn repeat(&self, i: usize, n: usize) -> Col {
        match self {
            Col::Num(v) => Col::Num(vec![v[i]; n]),
            Col::Cat(c, l) => Col::Cat(vec![c[i]; n], l.clone()),
        }
    }

    pub fn nums(&self) -> Option<&Vec<f64>> {
        match self {
            Col::Num(v) => Some(v),
            _ => None,
        }
    }

    pub fn is_na(&self, i: usize) -> bool {
        match self {
            Col::Num(v) => !v[i].is_finite(),
            Col::Cat(c, _) => c[i] == NA,
        }
    }

    /// A value's text (a level, or the number).
    pub fn text(&self, i: usize) -> String {
        match self {
            Col::Num(v) => fmt_value(v[i]),
            Col::Cat(c, l) => {
                if c[i] == NA {
                    "NA".to_string()
                } else {
                    l[c[i] as usize].clone()
                }
            }
        }
    }

    /// Concatenate columns of one kind (categorical levels merged).
    pub fn concat(parts: &[&Col]) -> Col {
        if parts.is_empty() {
            return Col::Num(vec![]);
        }
        if parts.iter().all(|p| matches!(p, Col::Num(_))) {
            let mut out = Vec::new();
            for p in parts {
                out.extend_from_slice(p.nums().unwrap());
            }
            return Col::Num(out);
        }
        // categorical: one level list in first-seen order
        let mut levels: Vec<String> = Vec::new();
        let mut index: BTreeMap<String, u32> = BTreeMap::new();
        let mut codes = Vec::new();
        for p in parts {
            match p {
                Col::Cat(c, l) => {
                    let map: Vec<u32> = l
                        .iter()
                        .map(|s| {
                            *index.entry(s.clone()).or_insert_with(|| {
                                levels.push(s.clone());
                                (levels.len() - 1) as u32
                            })
                        })
                        .collect();
                    codes.extend(c.iter().map(|&k| if k == NA { NA } else { map[k as usize] }));
                }
                Col::Num(v) => {
                    for x in v.iter() {
                        if !x.is_finite() {
                            codes.push(NA);
                            continue;
                        }
                        let s = fmt_value(*x);
                        let k = *index.entry(s.clone()).or_insert_with(|| {
                            levels.push(s);
                            (levels.len() - 1) as u32
                        });
                        codes.push(k);
                    }
                }
            }
        }
        Col::Cat(codes, Arc::new(levels))
    }
}

/// A number as a tooltip says it: up to six significant digits.
pub fn fmt_value(v: f64) -> String {
    if !v.is_finite() {
        return "NA".to_string();
    }
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let a = v.abs();
    if !(1e-4..1e9).contains(&a) {
        return format!("{:.4e}", v);
    }
    let mag = a.log10().floor() as i32;
    let d = (5 - mag).clamp(0, 10) as usize;
    let s = format!("{:.*}", d, v);
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    s
}

/// A layer's frame as the pipeline computes it.
#[derive(Clone, Debug, Default)]
pub struct LFrame {
    pub n: usize,
    pub cols: BTreeMap<Aes, Col>,
    /// The facet panel each row is drawn in.
    pub panel: Vec<u32>,
    /// The group each row belongs to (lines connect within a group).
    pub group: Vec<u32>,
    /// Variables a stat computed (`count`, `density`, …): mapped by
    /// `after_stat` and said by a tooltip.
    pub stat: BTreeMap<String, Vec<f64>>,
    /// A boxplot's outliers, one list per row.
    pub outliers: Option<Vec<Vec<f64>>>,
}

impl LFrame {
    pub fn take(&self, idx: &[usize]) -> LFrame {
        LFrame {
            n: idx.len(),
            cols: self.cols.iter().map(|(a, c)| (*a, c.take(idx))).collect(),
            panel: idx.iter().map(|&i| self.panel[i]).collect(),
            group: idx.iter().map(|&i| self.group[i]).collect(),
            stat: self
                .stat
                .iter()
                .map(|(k, v)| (k.clone(), idx.iter().map(|&i| v[i]).collect()))
                .collect(),
            outliers: self
                .outliers
                .as_ref()
                .map(|o| idx.iter().map(|&i| o[i].clone()).collect()),
        }
    }

    pub fn num(&self, a: Aes) -> Option<&Vec<f64>> {
        self.cols.get(&a).and_then(|c| c.nums())
    }

    pub fn set_num(&mut self, a: Aes, v: Vec<f64>) {
        self.cols.insert(a, Col::Num(v));
    }

    /// Concatenate frames (their columns' union; a column missing from a
    /// part is NA there).
    pub fn concat(parts: Vec<LFrame>) -> LFrame {
        if parts.len() == 1 {
            return parts.into_iter().next().unwrap();
        }
        let n: usize = parts.iter().map(|p| p.n).sum();
        let mut out = LFrame {
            n,
            ..Default::default()
        };
        let mut aes: Vec<Aes> = parts.iter().flat_map(|p| p.cols.keys().copied()).collect();
        aes.sort();
        aes.dedup();
        for a in aes {
            let filled: Vec<Col> = parts
                .iter()
                .map(|p| match p.cols.get(&a) {
                    Some(c) => c.clone(),
                    None => Col::Num(vec![f64::NAN; p.n]),
                })
                .collect();
            let refs: Vec<&Col> = filled.iter().collect();
            out.cols.insert(a, Col::concat(&refs));
        }
        let mut stats: Vec<String> = parts.iter().flat_map(|p| p.stat.keys().cloned()).collect();
        stats.sort();
        stats.dedup();
        for s in stats {
            let mut v = Vec::with_capacity(n);
            for p in &parts {
                match p.stat.get(&s) {
                    Some(x) => v.extend_from_slice(x),
                    None => v.extend(std::iter::repeat_n(f64::NAN, p.n)),
                }
            }
            out.stat.insert(s, v);
        }
        if parts.iter().any(|p| p.outliers.is_some()) {
            let mut o = Vec::with_capacity(n);
            for p in &parts {
                match &p.outliers {
                    Some(x) => o.extend(x.iter().cloned()),
                    None => o.extend(std::iter::repeat_n(Vec::new(), p.n)),
                }
            }
            out.outliers = Some(o);
        }
        for p in parts {
            out.panel.extend(p.panel);
            out.group.extend(p.group);
        }
        out
    }
}

/// An ISO date or datetime as seconds since 1970-01-01 (UTC, naive):
/// `YYYY-MM-DD`, `YYYY-MM`, `YYYY-MM-DD[T ]HH:MM[:SS[.fff]][Z]`.
/// Returns (seconds, has_time).
pub fn parse_iso(s: &str) -> Option<(f64, bool)> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 7 || b[4] != b'-' {
        return None;
    }
    let num = |a: usize, e: usize| -> Option<i64> {
        if e > b.len() {
            return None;
        }
        let t = &s[a..e];
        if t.bytes().all(|c| c.is_ascii_digit()) {
            t.parse().ok()
        } else {
            None
        }
    };
    let y = num(0, 4)?;
    let m = num(5, 7)?;
    if !(1..=12).contains(&m) {
        return None;
    }
    let (d, rest) = if b.len() == 7 {
        (1, "")
    } else {
        if b.len() < 10 || b[7] != b'-' {
            return None;
        }
        (num(8, 10)?, &s[10..])
    };
    if !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, m as u32, d as u32) as f64;
    if rest.is_empty() {
        return Some((days * 86400.0, false));
    }
    let r = rest.as_bytes();
    if !(r[0] == b'T' || r[0] == b' ') || r.len() < 6 || r[3] != b':' {
        return None;
    }
    let rs = &rest[1..];
    let hh: i64 = rs.get(0..2)?.parse().ok()?;
    let mm: i64 = rs.get(3..5)?.parse().ok()?;
    let mut sec = 0.0;
    let tail = &rs[5..];
    let tail = if let Some(t) = tail.strip_prefix(':') {
        let end = t
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(t.len());
        sec = t[..end].parse().ok()?;
        &t[end..]
    } else {
        tail
    };
    if !(tail.is_empty() || tail == "Z" || tail.starts_with('+') || tail.starts_with('-')) {
        return None;
    }
    Some((days * 86400.0 + (hh * 3600 + mm * 60) as f64 + sec, true))
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates_parse() {
        assert_eq!(parse_iso("1970-01-02"), Some((86400.0, false)));
        assert_eq!(parse_iso("2024-03-01").unwrap().0, 19783.0 * 86400.0);
        assert_eq!(parse_iso("2024-03").unwrap().0, 19783.0 * 86400.0);
        assert_eq!(parse_iso("1970-01-01T01:30").unwrap(), (5400.0, true));
        assert_eq!(parse_iso("1970-01-01 00:00:02.5Z").unwrap(), (2.5, true));
        assert!(parse_iso("hello").is_none());
        assert!(parse_iso("2024-13-01").is_none());
        assert_eq!(civil_from_days(19783), (2024, 3, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn values_format_compactly() {
        assert_eq!(fmt_value(3.0), "3");
        assert_eq!(fmt_value(3.14159265), "3.14159");
        assert_eq!(fmt_value(1234.5678), "1234.57");
        assert_eq!(fmt_value(f64::NAN), "NA");
    }
}
