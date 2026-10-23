//! Scales: how data values become positions, colours, sizes, shapes,
//! opacities and dash patterns, and the breaks and labels that explain
//! them (axes and legends).

use super::data::{civil_from_days, days_from_civil};
use super::scene::{Rgba, Shape};
use super::theme::{Appearance, quiet};
use super::{Param, Params, ScaleSpec, bad, p_num, p_str, p_strs};

// ── transformations ──────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Trans {
    #[default]
    Identity,
    Log10,
    Sqrt,
    Reverse,
}

impl Trans {
    pub fn apply(self, v: f64) -> f64 {
        match self {
            Trans::Identity => v,
            Trans::Log10 => {
                if v > 0.0 {
                    v.log10()
                } else {
                    f64::NAN
                }
            }
            Trans::Sqrt => {
                if v >= 0.0 {
                    v.sqrt()
                } else {
                    f64::NAN
                }
            }
            Trans::Reverse => -v,
        }
    }

    pub fn inverse(self, v: f64) -> f64 {
        match self {
            Trans::Identity => v,
            Trans::Log10 => 10f64.powf(v),
            Trans::Sqrt => v * v,
            Trans::Reverse => -v,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Trans::Identity => "identity",
            Trans::Log10 => "log10",
            Trans::Sqrt => "sqrt",
            Trans::Reverse => "reverse",
        }
    }
}

// ── breaks ───────────────────────────────────────────────────────────

/// About `n` nice breaks (1, 2, 2.5, 5 × 10^k) inside [lo, hi].
pub fn nice_breaks(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    if !(lo.is_finite() && hi.is_finite()) {
        return vec![];
    }
    if hi - lo <= 0.0 {
        return vec![lo];
    }
    let n = n.max(2) as f64;
    let raw = (hi - lo) / n;
    let mag = 10f64.powf(raw.log10().floor());
    let mut best: Option<(f64, f64)> = None;
    for m in [1.0, 2.0, 2.5, 5.0, 10.0] {
        let step = m * mag;
        let count = ((hi / step).floor() - (lo / step).ceil() + 1.0).max(0.0);
        // prefer counts near n; 2.5 only when nothing else fits as well
        let penalty = (count - n).abs() + if m == 2.5 { 1.5 } else { 0.0 };
        if count >= 2.0 && best.is_none_or(|(_, p)| penalty < p) {
            best = Some((step, penalty));
        }
    }
    let step = best.map(|b| b.0).unwrap_or(raw);
    let a = (lo / step - 1e-9).ceil();
    let b = (hi / step + 1e-9).floor();
    let mut out = Vec::new();
    let mut k = a;
    // snap each break to the step's decimals (0.6, not 0.6000000000000001)
    let d = ((-step.log10().floor()).max(0.0) + 3.0).min(15.0) as i32;
    let unit = 10f64.powi(d);
    while k <= b && out.len() < 100 {
        let v = (k * step * unit).round() / unit;
        out.push(if v.abs() < step * 1e-9 { 0.0 } else { v });
        k += 1.0;
    }
    out
}

/// The step of a list of breaks (for the labels' decimals).
fn step_of(b: &[f64]) -> f64 {
    if b.len() < 2 {
        return b
            .first()
            .map(|v| v.abs())
            .filter(|v| *v > 0.0)
            .unwrap_or(1.0);
    }
    let mut s = f64::INFINITY;
    for w in b.windows(2) {
        let d = (w[1] - w[0]).abs();
        if d > 0.0 {
            s = s.min(d);
        }
    }
    if s.is_finite() { s } else { 1.0 }
}

/// Breaks of a log10 axis over [lo, hi] (exponents): powers of ten,
/// with 2 and 5 between them over a short range.
pub fn log_breaks(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    let (a, b) = (lo.floor() as i64, hi.ceil() as i64);
    let decades = (b - a).max(1);
    let mut out = Vec::new();
    if decades as usize >= 3 {
        let every = ((decades as f64 / n.max(2) as f64).ceil() as i64).max(1);
        let mut e = a;
        while e <= b {
            if e % every == 0 || every == 1 {
                let v = e as f64;
                if v >= lo - 1e-9 && v <= hi + 1e-9 {
                    out.push(v);
                }
            }
            e += 1;
        }
    }
    if out.len() < 3 {
        out.clear();
        for e in a..=b {
            for m in [1.0, 2.0, 5.0] {
                let v = (m * 10f64.powi(e as i32)).log10();
                if v >= lo - 1e-9 && v <= hi + 1e-9 {
                    out.push(v);
                }
            }
        }
    }
    if out.len() < 2 {
        // under a decade: nice breaks on the raw values
        out = nice_breaks(10f64.powf(lo), 10f64.powf(hi), n)
            .into_iter()
            .filter(|v| *v > 0.0)
            .map(|v| v.log10())
            .collect();
    }
    out
}

// ── time ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeUnit {
    Sec(f64),
    Day(f64),
    Month(i64),
    Year(i64),
}

/// Breaks over [lo, hi] seconds: a calendar step giving about `n`.
pub fn time_breaks(lo: f64, hi: f64, n: usize, every: Option<TimeUnit>) -> (Vec<f64>, TimeUnit) {
    let span = (hi - lo).max(1.0);
    let n = n.max(2) as f64;
    let unit = every.unwrap_or_else(|| {
        let cands: [TimeUnit; 22] = [
            TimeUnit::Sec(1.0),
            TimeUnit::Sec(5.0),
            TimeUnit::Sec(15.0),
            TimeUnit::Sec(30.0),
            TimeUnit::Sec(60.0),
            TimeUnit::Sec(300.0),
            TimeUnit::Sec(900.0),
            TimeUnit::Sec(1800.0),
            TimeUnit::Sec(3600.0),
            TimeUnit::Sec(3.0 * 3600.0),
            TimeUnit::Sec(6.0 * 3600.0),
            TimeUnit::Sec(12.0 * 3600.0),
            TimeUnit::Day(1.0),
            TimeUnit::Day(2.0),
            TimeUnit::Day(7.0),
            TimeUnit::Day(14.0),
            TimeUnit::Month(1),
            TimeUnit::Month(3),
            TimeUnit::Month(6),
            TimeUnit::Year(1),
            TimeUnit::Year(2),
            TimeUnit::Year(5),
        ];
        let secs = |u: TimeUnit| match u {
            TimeUnit::Sec(s) => s,
            TimeUnit::Day(d) => d * 86400.0,
            TimeUnit::Month(m) => m as f64 * 30.44 * 86400.0,
            TimeUnit::Year(y) => y as f64 * 365.25 * 86400.0,
        };
        for u in cands {
            if span / secs(u) <= n * 1.25 {
                return u;
            }
        }
        // decades and centuries
        let years = span / (365.25 * 86400.0) / n;
        let mag = 10f64.powf(years.log10().floor());
        let m = [1.0, 2.0, 5.0, 10.0]
            .into_iter()
            .find(|m| m * mag >= years)
            .unwrap_or(10.0);
        TimeUnit::Year((m * mag).max(1.0) as i64)
    });
    let mut out = Vec::new();
    match unit {
        TimeUnit::Sec(_) | TimeUnit::Day(_) => {
            let step = match unit {
                TimeUnit::Day(d) => d * 86400.0,
                TimeUnit::Sec(s) => s,
                _ => unreachable!(),
            };
            // weeks start on Monday (1970-01-05 was one)
            let origin = if matches!(unit, TimeUnit::Day(d) if d >= 7.0) {
                4.0 * 86400.0
            } else {
                0.0
            };
            let mut t = ((lo - origin) / step).ceil() * step + origin;
            while t <= hi + 1e-6 && out.len() < 200 {
                out.push(t);
                t += step;
            }
        }
        TimeUnit::Month(k) => {
            let (y0, m0, _) = civil_from_days((lo / 86400.0).floor() as i64);
            let mut y = y0;
            let mut m = m0 as i64;
            // align to the step from January
            while (m - 1) % k != 0 {
                m += 1;
                if m > 12 {
                    m = 1;
                    y += 1;
                }
            }
            loop {
                let t = days_from_civil(y, m as u32, 1) as f64 * 86400.0;
                if t > hi + 1e-6 || out.len() > 400 {
                    break;
                }
                if t >= lo - 1e-6 {
                    out.push(t);
                }
                m += k;
                while m > 12 {
                    m -= 12;
                    y += 1;
                }
            }
        }
        TimeUnit::Year(k) => {
            let (y0, _, _) = civil_from_days((lo / 86400.0).floor() as i64);
            let mut y = (y0 as f64 / k as f64).floor() as i64 * k;
            loop {
                let t = days_from_civil(y, 1, 1) as f64 * 86400.0;
                if t > hi + 1e-6 || out.len() > 400 {
                    break;
                }
                if t >= lo - 1e-6 {
                    out.push(t);
                }
                y += k;
            }
        }
    }
    (out, unit)
}

const MONTHS: [&str; 12] = [
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
const DAYS: [&str; 7] = [
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
];

/// A time (seconds) formatted strftime-style: %Y %y %m %d %e %b %B %H %M
/// %S %j %a %A %%.
pub fn strftime(t: f64, fmt: &str) -> String {
    let days = (t / 86400.0).floor() as i64;
    let secs = (t - days as f64 * 86400.0).round() as i64;
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (secs / 3600, (secs / 60) % 60, secs % 60);
    let mut out = String::new();
    let mut it = fmt.chars();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('Y') => out.push_str(&y.to_string()),
            Some('y') => out.push_str(&format!("{:02}", y.rem_euclid(100))),
            Some('m') => out.push_str(&format!("{:02}", m)),
            Some('d') => out.push_str(&format!("{:02}", d)),
            Some('e') => out.push_str(&d.to_string()),
            Some('b') => out.push_str(&MONTHS[(m - 1) as usize][..3]),
            Some('B') => out.push_str(MONTHS[(m - 1) as usize]),
            Some('H') => out.push_str(&format!("{:02}", hh)),
            Some('M') => out.push_str(&format!("{:02}", mm)),
            Some('S') => out.push_str(&format!("{:02}", ss)),
            Some('j') => {
                let doy = days - days_from_civil(y, 1, 1) + 1;
                out.push_str(&format!("{:03}", doy))
            }
            Some('a') => out.push_str(&DAYS[days.rem_euclid(7) as usize][..3]),
            Some('A') => out.push_str(DAYS[days.rem_euclid(7) as usize]),
            Some('%') => out.push('%'),
            Some(o) => {
                out.push('%');
                out.push(o);
            }
            None => out.push('%'),
        }
    }
    out
}

fn time_labels(breaks: &[f64], unit: TimeUnit, datetime: bool) -> Vec<String> {
    let years: Vec<i64> = breaks
        .iter()
        .map(|t| civil_from_days((t / 86400.0).floor() as i64).0)
        .collect();
    let spans_years = years.first() != years.last();
    let fmt = match unit {
        TimeUnit::Year(_) => "%Y",
        TimeUnit::Month(_) => {
            if spans_years {
                "%b %Y"
            } else {
                "%b"
            }
        }
        TimeUnit::Day(_) => "%b %e",
        TimeUnit::Sec(s) if s < 60.0 => "%H:%M:%S",
        TimeUnit::Sec(_) => {
            let days: Vec<i64> = breaks
                .iter()
                .map(|t| (t / 86400.0).floor() as i64)
                .collect();
            if days.first() != days.last() {
                "%b %e %H:%M"
            } else {
                "%H:%M"
            }
        }
    };
    let _ = datetime;
    let mut out: Vec<String> = breaks.iter().map(|t| strftime(*t, fmt)).collect();
    // a month axis over years says the year once, where it changes
    if fmt == "%b %Y" {
        let mut last = i64::MIN;
        for (i, t) in breaks.iter().enumerate() {
            let y = years[i];
            if y == last {
                out[i] = strftime(*t, "%b");
            }
            last = y;
        }
    }
    out
}

// ── number labels ────────────────────────────────────────────────────

fn group3(int_part: &str) -> String {
    let neg = int_part.starts_with('-');
    let digits = int_part.trim_start_matches('-');
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if neg { format!("-{}", out) } else { out }
}

fn decimals_for(step: f64) -> usize {
    if step <= 0.0 || !step.is_finite() {
        return 0;
    }
    let mut d = 0usize;
    let mut s = step;
    while d < 8 && (s - s.round()).abs() > 1e-6 * s.max(1.0) {
        s *= 10.0;
        d += 1;
    }
    d
}

fn fixed(v: f64, d: usize, commas: bool) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    let s = format!("{:.*}", d, v);
    let s = if s == "-0"
        || s.starts_with("-0.") && s.trim_start_matches("-0.").chars().all(|c| c == '0')
    {
        s.trim_start_matches('-').to_string()
    } else {
        s
    };
    if !commas {
        return s;
    }
    match s.split_once('.') {
        Some((i, f)) => format!("{}.{}", group3(i), f),
        None => group3(&s),
    }
}

/// A number label style.
#[derive(Clone, Debug, PartialEq)]
pub enum NumFmt {
    Auto,
    Comma,
    Percent,
    Dollar,
    Short,
    Scientific,
    Plain,
}

impl NumFmt {
    pub fn parse(s: &str) -> Option<NumFmt> {
        Some(match s {
            "auto" => NumFmt::Auto,
            "comma" | "number" => NumFmt::Comma,
            "percent" | "pct" => NumFmt::Percent,
            "dollar" | "currency" => NumFmt::Dollar,
            "short" | "si" => NumFmt::Short,
            "scientific" | "sci" => NumFmt::Scientific,
            "plain" => NumFmt::Plain,
            _ => return None,
        })
    }
}

/// Labels for numbers (already back-transformed) at breaks of `step`.
pub fn format_numbers(
    vals: &[f64],
    fmt: &NumFmt,
    step: f64,
    prefix: &str,
    suffix: &str,
) -> Vec<String> {
    let max = vals.iter().fold(0f64, |a, v| a.max(v.abs()));
    vals.iter()
        .map(|&v| {
            let body = match fmt {
                NumFmt::Plain => fixed(v, decimals_for(step), false),
                NumFmt::Comma => fixed(v, decimals_for(step), true),
                NumFmt::Auto => {
                    if max >= 1e15 || (max < 1e-4 && max > 0.0) {
                        format!("{:e}", v)
                    } else {
                        fixed(v, decimals_for(step), max >= 1e4)
                    }
                }
                NumFmt::Percent => {
                    format!("{}%", fixed(v * 100.0, decimals_for(step * 100.0), true))
                }
                NumFmt::Dollar => {
                    let d = if step < 1.0 {
                        decimals_for(step).max(2)
                    } else {
                        0
                    };
                    let t = fixed(v.abs(), d, true);
                    if v < 0.0 {
                        format!("-${}", t)
                    } else {
                        format!("${}", t)
                    }
                }
                NumFmt::Short => {
                    // each value in its own unit, its zeros trimmed (200k, 1.5M)
                    let a = v.abs();
                    let (u, sfx) = if a >= 1e12 {
                        (1e12, "T")
                    } else if a >= 1e9 {
                        (1e9, "B")
                    } else if a >= 1e6 {
                        (1e6, "M")
                    } else if a >= 1e3 {
                        (1e3, "k")
                    } else {
                        (1.0, "")
                    };
                    let t = fixed(v / u, decimals_for(step / u).max(2), false);
                    let t = if t.contains('.') {
                        t.trim_end_matches('0').trim_end_matches('.').to_string()
                    } else {
                        t
                    };
                    if v == 0.0 {
                        "0".to_string()
                    } else {
                        format!("{}{}", t, sfx)
                    }
                }
                NumFmt::Scientific => {
                    if v == 0.0 {
                        "0".to_string()
                    } else {
                        let e = v.abs().log10().floor() as i32;
                        let m = v / 10f64.powi(e);
                        format!(
                            "{}e{}",
                            fixed(m, if (m - m.round()).abs() < 1e-9 { 0 } else { 1 }, false),
                            e
                        )
                    }
                }
            };
            format!("{}{}{}", prefix, body, suffix)
        })
        .collect()
}

// ── position scales ──────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Labels {
    Auto,
    Num(NumFmt),
    List(Vec<String>),
    Pairs(Vec<(String, String)>),
    Date(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Breaks {
    Auto,
    N(usize),
    At(Vec<f64>),
    AtLevels(Vec<String>),
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PosKind {
    Continuous,
    Discrete,
    Time { datetime: bool },
}

/// A position scale as specified (one per axis), trained by the build.
#[derive(Clone, Debug)]
pub struct PosScale {
    pub axis: char,
    pub kind: PosKind,
    pub trans: Trans,
    pub name: Option<String>,
    pub limits: Option<(Option<f64>, Option<f64>)>,
    pub dlimits: Option<Vec<String>>,
    pub breaks: Breaks,
    pub labels: Labels,
    pub expand: Option<(f64, f64)>,
    pub every: Option<TimeUnit>,
    pub prefix: String,
    pub suffix: String,
    /// Whether the user named the kind (a discrete or date scale).
    pub forced: bool,
    /// The discrete levels, once trained.
    pub levels: Vec<String>,
}

/// One panel's trained axis.
#[derive(Clone, Debug)]
pub struct Trained {
    /// The ends shown, in the transformed space.
    pub lo: f64,
    pub hi: f64,
    /// Breaks in the transformed space, and their labels.
    pub breaks: Vec<f64>,
    pub labels: Vec<String>,
}

impl PosScale {
    pub fn new(axis: char) -> PosScale {
        PosScale {
            axis,
            kind: PosKind::Continuous,
            trans: Trans::Identity,
            name: None,
            limits: None,
            dlimits: None,
            breaks: Breaks::Auto,
            labels: Labels::Auto,
            expand: None,
            every: None,
            prefix: String::new(),
            suffix: String::new(),
            forced: false,
            levels: Vec::new(),
        }
    }

    /// Apply a `scale_x_*` spec.
    pub fn configure(&mut self, s: &ScaleSpec) -> super::Result<()> {
        let p = &s.params;
        match s.kind.as_str() {
            "continuous" => {}
            "log10" | "log" => self.trans = Trans::Log10,
            "sqrt" => self.trans = Trans::Sqrt,
            "reverse" => self.trans = Trans::Reverse,
            "discrete" => {
                self.kind = PosKind::Discrete;
                self.forced = true;
            }
            "date" => {
                self.kind = PosKind::Time { datetime: false };
                self.forced = true;
            }
            "datetime" | "time" => {
                self.kind = PosKind::Time { datetime: true };
                self.forced = true;
            }
            other => return bad(format!("scale_{}: unknown kind \"{}\"", self.axis, other)),
        }
        if let Some(t) = p_str(p, "trans") {
            self.trans = match t {
                "identity" => Trans::Identity,
                "log10" | "log" => Trans::Log10,
                "sqrt" => Trans::Sqrt,
                "reverse" => Trans::Reverse,
                o => return bad(format!("scale_{}: unknown trans \"{}\"", self.axis, o)),
            };
        }
        if let Some(n) = p_str(p, "name") {
            self.name = Some(n.to_string());
        }
        match p.get("limits") {
            Some(Param::Nums(v)) if v.len() == 2 => {
                let f = |x: f64| if x.is_finite() { Some(x) } else { None };
                self.limits = Some((f(v[0]), f(v[1])));
            }
            Some(Param::Strs(v)) => self.dlimits = Some(v.clone()),
            Some(_) => {
                return bad(format!(
                    "scale_{}: limits is [lo, hi] (either may be ()), or a list of levels",
                    self.axis
                ));
            }
            None => {}
        }
        match p.get("breaks") {
            Some(Param::Nums(v)) => self.breaks = Breaks::At(v.clone()),
            Some(Param::Num(n)) => self.breaks = Breaks::N((*n).max(1.0) as usize),
            Some(Param::Strs(v)) => self.breaks = Breaks::AtLevels(v.clone()),
            Some(Param::Bool(false)) => self.breaks = Breaks::None,
            Some(Param::Str(s)) if s == "none" => self.breaks = Breaks::None,
            Some(_) => {
                return bad(format!(
                    "scale_{}: breaks is a list of values or a count",
                    self.axis
                ));
            }
            None => {}
        }
        if let Some(every) = p_str(p, "date_breaks") {
            self.every = Some(parse_every(every)?);
        }
        match p.get("labels") {
            Some(Param::Str(f)) => {
                self.labels = match NumFmt::parse(f) {
                    Some(n) => Labels::Num(n),
                    None if f.contains('%') => Labels::Date(f.clone()),
                    None => {
                        return bad(format!(
                            "scale_{}: labels \"{}\" (percent, comma, dollar, short, scientific, plain, a date format like \"%b %Y\", or a list)",
                            self.axis, f
                        ));
                    }
                }
            }
            Some(Param::Strs(v)) => self.labels = Labels::List(v.clone()),
            Some(Param::Pairs(v)) => self.labels = Labels::Pairs(v.clone()),
            Some(_) => {
                return bad(format!(
                    "scale_{}: labels is a format name or a list",
                    self.axis
                ));
            }
            None => {}
        }
        if let Some(f) = p_str(p, "date_labels") {
            self.labels = Labels::Date(f.to_string());
        }
        if let Some(Param::Nums(v)) = p.get("expand")
            && v.len() == 2
        {
            self.expand = Some((v[0], v[1]));
        }
        if let Some(s) = p_str(p, "prefix") {
            self.prefix = s.to_string();
        }
        if let Some(s) = p_str(p, "suffix") {
            self.suffix = s.to_string();
        }
        Ok(())
    }

    /// Train an axis over a range (transformed space) of data `dlo..dhi`:
    /// limits, expansion, breaks and labels for a panel `len` pixels long.
    pub fn train(
        &self,
        dlo: f64,
        dhi: f64,
        len: f64,
        zero_floor: bool,
        zoom: Option<(f64, f64)>,
    ) -> Trained {
        self.train_on(dlo, dhi, len, self.axis == 'x', false, zero_floor, zoom)
    }

    /// Train drawn `horizontal`ly or not (the breaks' spacing), over
    /// whole-numbered data or not (no breaks between whole numbers).
    #[allow(clippy::too_many_arguments)]
    pub fn train_on(
        &self,
        dlo: f64,
        dhi: f64,
        len: f64,
        horizontal: bool,
        whole: bool,
        zero_floor: bool,
        zoom: Option<(f64, f64)>,
    ) -> Trained {
        let (mut lo, mut hi) = (dlo, dhi);
        if let Some((a, b)) = self.limits {
            if let Some(a) = a {
                lo = self.trans.apply(a);
            }
            if let Some(b) = b {
                hi = self.trans.apply(b);
            }
            if self.trans == Trans::Reverse {
                std::mem::swap(&mut lo, &mut hi);
            }
        }
        if !(lo.is_finite() && hi.is_finite()) {
            lo = 0.0;
            hi = 1.0;
        }
        if lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        let discrete = self.kind == PosKind::Discrete;
        if let Some((a, b)) = zoom {
            lo = self.trans.apply(a).min(self.trans.apply(b));
            hi = self.trans.apply(a).max(self.trans.apply(b));
        } else {
            // expansion: 5% continuous, 0.6 of a level discrete; none on a
            // bar's floor
            let (mult, add) =
                self.expand
                    .unwrap_or(if discrete { (0.0, 0.6) } else { (0.05, 0.0) });
            if hi - lo < 1e-12 {
                let d = if lo.abs() > 0.0 { lo.abs() * 0.1 } else { 0.5 };
                lo -= d;
                hi += d;
            }
            let span = hi - lo;
            let lo_floor =
                zero_floor && lo == 0.0 && self.expand.is_none() && self.limits.is_none();
            if !lo_floor {
                lo -= span * mult + add;
            }
            hi += span * mult + add;
        }
        let target = if horizontal {
            (len / 90.0).round().clamp(2.0, 10.0)
        } else {
            (len / 42.0).round().clamp(3.0, 10.0)
        } as usize;
        let (mut breaks, mut labels) = self.breaks_and_labels(lo, hi, target);
        // whole-numbered data (counts, years, bedrooms) never breaks between whole numbers
        if whole
            && self.kind == PosKind::Continuous
            && self.trans == Trans::Identity
            && self.breaks == Breaks::Auto
            && breaks.len() > 1
            && breaks.iter().any(|b| b.fract() != 0.0)
        {
            let a = lo.ceil() as i64;
            let b = hi.floor() as i64;
            let span = (b - a).max(1);
            let step = [1i64, 2, 5, 10, 20, 25, 50]
                .into_iter()
                .find(|s| span / s <= target as i64)
                .unwrap_or(1);
            breaks = (a..=b)
                .filter(|v| v % step == 0)
                .map(|v| v as f64)
                .collect();
            labels = self.number_labels(&breaks);
        }
        Trained {
            lo,
            hi,
            breaks,
            labels,
        }
    }

    fn breaks_and_labels(&self, lo: f64, hi: f64, n: usize) -> (Vec<f64>, Vec<String>) {
        if self.breaks == Breaks::None {
            return (vec![], vec![]);
        }
        if self.kind == PosKind::Discrete {
            let mut bs = Vec::new();
            let mut ls = Vec::new();
            for (i, l) in self.levels.iter().enumerate() {
                let v = (i + 1) as f64;
                if let Breaks::AtLevels(keep) = &self.breaks
                    && !keep.contains(l)
                {
                    continue;
                }
                if v >= lo - 1e-9 && v <= hi + 1e-9 {
                    bs.push(v);
                    ls.push(self.relabel(l, i));
                }
            }
            return (bs, ls);
        }
        if let PosKind::Time { datetime } = self.kind {
            let (bs, unit) = match &self.breaks {
                Breaks::At(v) => (v.clone(), TimeUnit::Day(1.0)),
                Breaks::N(k) => time_breaks(lo, hi, *k, self.every),
                _ => time_breaks(lo, hi, n, self.every),
            };
            let bs: Vec<f64> = bs
                .into_iter()
                .filter(|v| *v >= lo - 1e-6 && *v <= hi + 1e-6)
                .collect();
            let ls = match &self.labels {
                Labels::Date(f) => bs.iter().map(|t| strftime(*t, f)).collect(),
                Labels::List(l) => (0..bs.len())
                    .map(|i| l.get(i).cloned().unwrap_or_default())
                    .collect(),
                _ => time_labels(&bs, unit, datetime),
            };
            return (bs, ls);
        }
        let raw: Vec<f64> = match &self.breaks {
            Breaks::At(v) => v
                .iter()
                .map(|x| self.trans.apply(*x))
                .filter(|v| v.is_finite() && *v >= lo - 1e-9 && *v <= hi + 1e-9)
                .collect(),
            other => {
                let n = if let Breaks::N(k) = other { *k } else { n };
                match self.trans {
                    Trans::Log10 => log_breaks(lo, hi, n),
                    Trans::Sqrt => nice_breaks(lo.max(0.0).powi(2), hi.powi(2), n)
                        .into_iter()
                        .map(|v| v.sqrt())
                        .collect(),
                    Trans::Reverse => {
                        let mut b: Vec<f64> =
                            nice_breaks(-hi, -lo, n).into_iter().map(|v| -v).collect();
                        b.reverse();
                        b
                    }
                    Trans::Identity => nice_breaks(lo, hi, n),
                }
            }
        };
        let vals: Vec<f64> = raw.iter().map(|v| self.trans.inverse(*v)).collect();
        let labels = self.number_labels(&vals);
        (raw, labels)
    }

    fn relabel(&self, level: &str, i: usize) -> String {
        match &self.labels {
            Labels::List(l) => l.get(i).cloned().unwrap_or_else(|| level.to_string()),
            Labels::Pairs(p) => p
                .iter()
                .find(|(k, _)| k == level)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| level.to_string()),
            _ => level.to_string(),
        }
    }

    pub fn number_labels(&self, vals: &[f64]) -> Vec<String> {
        match &self.labels {
            Labels::List(l) => (0..vals.len())
                .map(|i| l.get(i).cloned().unwrap_or_default())
                .collect(),
            Labels::Num(f) => format_numbers(vals, f, step_of(vals), &self.prefix, &self.suffix),
            _ => {
                let f = if self.trans == Trans::Log10 {
                    let max = vals.iter().fold(0f64, |a, v| a.max(v.abs()));
                    if max >= 1e6 {
                        NumFmt::Short
                    } else {
                        NumFmt::Comma
                    }
                } else {
                    NumFmt::Auto
                };
                let step = if self.trans == Trans::Log10 {
                    vals.iter()
                        .copied()
                        .filter(|v| *v > 0.0)
                        .fold(f64::INFINITY, f64::min)
                } else {
                    step_of(vals)
                };
                format_numbers(vals, &f, step, &self.prefix, &self.suffix)
            }
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            PosKind::Continuous => "continuous",
            PosKind::Discrete => "discrete",
            PosKind::Time { .. } => "time",
        }
    }
}

fn parse_every(s: &str) -> super::Result<TimeUnit> {
    let mut it = s.split_whitespace();
    let (k, unit) = match (it.next(), it.next()) {
        (Some(a), Some(b)) => (a.parse::<f64>().unwrap_or(1.0), b),
        (Some(a), None) => (1.0, a),
        _ => return bad("date_breaks: like \"1 month\", \"2 weeks\", \"6 hours\""),
    };
    let u = unit.trim_end_matches('s');
    Ok(match u {
        "sec" | "second" => TimeUnit::Sec(k),
        "min" | "minute" => TimeUnit::Sec(k * 60.0),
        "hour" => TimeUnit::Sec(k * 3600.0),
        "day" => TimeUnit::Day(k),
        "week" => TimeUnit::Day(k * 7.0),
        "month" => TimeUnit::Month(k.max(1.0) as i64),
        "quarter" => TimeUnit::Month((k * 3.0) as i64),
        "year" => TimeUnit::Year(k.max(1.0) as i64),
        _ => return bad(format!("date_breaks: unknown unit \"{}\"", unit)),
    })
}

// ── colours ──────────────────────────────────────────────────────────

/// A colour by name (CSS's common ones, R's greys) or `#hex`.
pub fn color(s: &str) -> Option<Rgba> {
    if s.starts_with('#') {
        return Rgba::hex(s);
    }
    let lower = s.to_ascii_lowercase().replace([' ', '_'], "");
    if let Some(rest) = lower
        .strip_prefix("gray")
        .or_else(|| lower.strip_prefix("grey"))
        && let Ok(k) = rest.parse::<u8>()
    {
        let v = (k.min(100) as f64 * 2.55).round() as u8;
        return Some(Rgba::new(v, v, v));
    }
    let hex = match lower.as_str() {
        "black" => "#000000",
        "white" => "#ffffff",
        "gray" | "grey" => "#808080",
        "darkgray" | "darkgrey" => "#a9a9a9",
        "lightgray" | "lightgrey" => "#d3d3d3",
        "dimgray" | "dimgrey" => "#696969",
        "red" => "#e5342f",
        "darkred" => "#8b0000",
        "firebrick" => "#b22222",
        "tomato" => "#ff6347",
        "salmon" => "#fa8072",
        "orange" => "#ffa500",
        "darkorange" => "#ff8c00",
        "gold" => "#ffd700",
        "yellow" => "#ffff00",
        "khaki" => "#f0e68c",
        "green" => "#2e9e44",
        "darkgreen" => "#006400",
        "forestgreen" => "#228b22",
        "seagreen" => "#2e8b57",
        "olivedrab" => "#6b8e23",
        "lime" => "#00ff00",
        "teal" => "#008080",
        "cyan" | "aqua" => "#00ffff",
        "turquoise" => "#40e0d0",
        "blue" => "#2f6fd6",
        "navy" => "#000080",
        "darkblue" => "#00008b",
        "steelblue" => "#4682b4",
        "royalblue" => "#4169e1",
        "dodgerblue" => "#1e90ff",
        "skyblue" => "#87ceeb",
        "lightblue" => "#add8e6",
        "purple" => "#800080",
        "violet" => "#ee82ee",
        "orchid" => "#da70d6",
        "magenta" | "fuchsia" => "#ff00ff",
        "pink" => "#ffc0cb",
        "hotpink" => "#ff69b4",
        "brown" => "#a52a2a",
        "sienna" => "#a0522d",
        "chocolate" => "#d2691e",
        "tan" => "#d2b48c",
        "beige" => "#f5f5dc",
        "maroon" => "#800000",
        "olive" => "#808000",
        "indigo" => "#4b0082",
        "coral" => "#ff7f50",
        "transparent" => "#00000000",
        _ => return None,
    };
    Rgba::hex(hex)
}

pub fn color_or_err(s: &str) -> super::Result<Rgba> {
    color(s).map_or_else(
        || {
            bad(format!(
                "unknown colour \"{}\" (a name like \"steelblue\" or \"#3a6fd8\")",
                s
            ))
        },
        Ok,
    )
}

/// The viridis family, sampled at eleven stops each (interpolated).
fn viridis_stops(option: &str) -> Option<&'static [&'static str]> {
    Some(match option {
        "viridis" | "D" => &[
            "#440154", "#482475", "#414487", "#355f8d", "#2a788e", "#21918c", "#22a884", "#44bf70",
            "#7ad151", "#bddf26", "#fde725",
        ],
        "magma" | "A" => &[
            "#000004", "#140e36", "#3b0f70", "#641a80", "#8c2981", "#b73779", "#de4968", "#f7705c",
            "#fe9f6d", "#fecf92", "#fcfdbf",
        ],
        "inferno" | "B" => &[
            "#000004", "#160b39", "#420a68", "#6a176e", "#932667", "#bc3754", "#dd513a", "#f37819",
            "#fca50a", "#f6d746", "#fcffa4",
        ],
        "plasma" | "C" => &[
            "#0d0887", "#41049d", "#6a00a8", "#8f0da4", "#b12a90", "#cc4778", "#e16462", "#f2844b",
            "#fca636", "#fcce25", "#f0f921",
        ],
        "cividis" | "E" => &[
            "#00204d", "#00306f", "#2a406c", "#48526b", "#5e626e", "#727374", "#878479", "#9e9677",
            "#b6a971", "#d0bd67", "#ffea46",
        ],
        "mako" | "G" => &[
            "#0b0405", "#231526", "#35264c", "#403a75", "#3d5296", "#366da0", "#3487a6", "#35a1ab",
            "#49b9ad", "#7ccfa8", "#def5e5",
        ],
        "rocket" | "F" => &[
            "#03051a", "#221331", "#451c47", "#6a1f56", "#921c5b", "#b91657", "#d92847", "#ed513e",
            "#f47c56", "#f6a47b", "#faebdd",
        ],
        _ => return None,
    })
}

/// A ramp: colours interpolated over stops, t in [0, 1].
#[derive(Clone, Debug)]
pub struct Ramp(pub Vec<Rgba>);

impl Ramp {
    pub fn at(&self, t: f64) -> Rgba {
        let s = &self.0;
        if s.len() == 1 {
            return s[0];
        }
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else {
            0.0
        } * (s.len() - 1) as f64;
        let i = (t.floor() as usize).min(s.len() - 2);
        s[i].mix(s[i + 1], t - i as f64)
    }

    pub fn viridis(option: &str, begin: f64, end: f64, reverse: bool) -> super::Result<Ramp> {
        let stops = viridis_stops(option).ok_or_else(|| {
            crate::OdsError::InvalidArgument(format!(
                "gg: viridis option \"{}\" (viridis, magma, inferno, plasma, cividis, mako, rocket)",
                option
            ))
        })?;
        let full = Ramp(stops.iter().map(|h| Rgba::hex(h).unwrap()).collect());
        let mut out: Vec<Rgba> = (0..=20)
            .map(|i| full.at(begin + (end - begin) * i as f64 / 20.0))
            .collect();
        if reverse {
            out.reverse();
        }
        Ok(Ramp(out))
    }
}

/// A non-position scale's mapping, trained.
#[derive(Clone, Debug)]
pub enum NpMap {
    /// Levels to visual values.
    Discrete {
        levels: Vec<String>,
        labels: Vec<String>,
    },
    /// A continuous range (transformed space) to [0, 1].
    Continuous {
        lo: f64,
        hi: f64,
        mid: Option<f64>,
        breaks: Vec<f64>,
        labels: Vec<String>,
    },
    /// Values are the visual values themselves.
    Identity,
}

/// A non-position scale: colour, fill, size, alpha, shape or linetype.
#[derive(Clone, Debug)]
pub struct NpScale {
    pub aes: super::Aes,
    pub kind: String,
    pub params: Params,
    pub name: Option<String>,
    pub map: Option<NpMap>,
    pub trans: Trans,
    /// The discrete colours (palette, manual values), or the ramp.
    pub palette: Vec<Rgba>,
    pub manual: Vec<(String, String)>,
    pub ramp: Option<Ramp>,
    pub guide: bool,
}

impl NpScale {
    pub fn new(aes: super::Aes) -> NpScale {
        NpScale {
            aes,
            kind: String::new(),
            params: Params::new(),
            name: None,
            map: None,
            trans: Trans::Identity,
            palette: vec![],
            manual: vec![],
            ramp: None,
            guide: true,
        }
    }

    pub fn configure(&mut self, s: &ScaleSpec) -> super::Result<()> {
        self.kind = s.kind.clone();
        self.params = s.params.clone();
        if let Some(n) = p_str(&s.params, "name") {
            self.name = Some(n.to_string());
        }
        if p_str(&s.params, "guide") == Some("none")
            || s.params.get("guide") == Some(&Param::Bool(false))
        {
            self.guide = false;
        }
        if let Some(t) = p_str(&s.params, "trans") {
            self.trans = match t {
                "log10" => Trans::Log10,
                "sqrt" => Trans::Sqrt,
                "reverse" => Trans::Reverse,
                _ => Trans::Identity,
            };
        }
        if let Some(Param::Pairs(p)) = s.params.get("values") {
            self.manual = p.clone();
        }
        Ok(())
    }

    /// Whether a mapping with these values is discrete under this scale.
    pub fn wants_discrete(&self, data_discrete: bool) -> bool {
        match self.kind.as_str() {
            "manual" | "viridis_d" | "palette" | "grey" | "gray" | "discrete" | "shape"
            | "linetype" => true,
            "viridis_c" | "gradient" | "gradient2" | "gradientn" | "continuous" | "diverging"
            | "area" | "radius" | "linewidth" => false,
            _ => data_discrete,
        }
    }

    /// Train a discrete mapping over levels, choosing its visual values.
    pub fn train_discrete(&mut self, levels: Vec<String>, app: Appearance) -> super::Result<()> {
        let labels = match self.params.get("labels") {
            Some(Param::Strs(l)) => (0..levels.len())
                .map(|i| l.get(i).cloned().unwrap_or_else(|| levels[i].clone()))
                .collect(),
            Some(Param::Pairs(p)) => levels
                .iter()
                .map(|lv| {
                    p.iter()
                        .find(|(k, _)| k == lv)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_else(|| lv.clone())
                })
                .collect(),
            _ => levels.clone(),
        };
        let n = levels.len().max(1);
        if matches!(self.aes, super::Aes::Color | super::Aes::Fill) {
            self.palette = match self.kind.as_str() {
                "manual" => {
                    let mut out = Vec::new();
                    let values = p_strs(&self.params, "values").unwrap_or_default();
                    for (i, lv) in levels.iter().enumerate() {
                        let c = if let Some((_, c)) = self.manual.iter().find(|(k, _)| k == lv) {
                            color_or_err(c)?
                        } else if !values.is_empty() {
                            color_or_err(&values[i % values.len()])?
                        } else {
                            Rgba::hex(quiet(app)[i % 10]).unwrap()
                        };
                        out.push(c);
                    }
                    out
                }
                "viridis_d" | "viridis" => {
                    let ramp = Ramp::viridis(
                        p_str(&self.params, "option").unwrap_or("viridis"),
                        p_num(&self.params, "begin").unwrap_or(0.0),
                        p_num(&self.params, "end").unwrap_or(if n > 1 { 0.92 } else { 0.0 }),
                        p_num(&self.params, "direction").unwrap_or(1.0) < 0.0,
                    )?;
                    (0..n)
                        .map(|i| {
                            ramp.at(if n == 1 {
                                0.0
                            } else {
                                i as f64 / (n - 1) as f64
                            })
                        })
                        .collect()
                }
                "grey" | "gray" => {
                    let a = p_num(&self.params, "start").unwrap_or(0.25);
                    let b = p_num(&self.params, "end").unwrap_or(0.75);
                    (0..n)
                        .map(|i| {
                            let t = if n == 1 {
                                a
                            } else {
                                a + (b - a) * i as f64 / (n - 1) as f64
                            };
                            let v = (t * 255.0).round() as u8;
                            Rgba::new(v, v, v)
                        })
                        .collect()
                }
                "palette" => {
                    let name = p_str(&self.params, "palette").unwrap_or("quiet");
                    if name == "quiet" {
                        quiet_n(app, n)?
                    } else {
                        let ramp = Ramp::viridis(name, 0.0, 0.92, false)?;
                        (0..n)
                            .map(|i| {
                                ramp.at(if n == 1 {
                                    0.0
                                } else {
                                    i as f64 / (n - 1) as f64
                                })
                            })
                            .collect()
                    }
                }
                _ => quiet_n(app, n)?,
            };
        }
        self.map = Some(NpMap::Discrete { levels, labels });
        Ok(())
    }

    /// Train a continuous mapping over a range (transformed space).
    pub fn train_continuous(&mut self, lo: f64, hi: f64, app: Appearance) -> super::Result<()> {
        let (mut lo, mut hi) = (lo, hi);
        if let Some(Param::Nums(l)) = self.params.get("limits")
            && l.len() == 2
        {
            if l[0].is_finite() {
                lo = self.trans.apply(l[0]);
            }
            if l[1].is_finite() {
                hi = self.trans.apply(l[1]);
            }
        }
        if !(lo.is_finite() && hi.is_finite()) {
            lo = 0.0;
            hi = 1.0;
        }
        let mut mid = None;
        if matches!(self.aes, super::Aes::Color | super::Aes::Fill) {
            let p = &self.params;
            let c = |k: &str, d: &str| color_or_err(p_str(p, k).unwrap_or(d));
            let (neutral, cold, warm) = match app {
                Appearance::Dark => ("#2c3038", "#6c9cf2", "#f2955e"),
                Appearance::Contrast => ("#f4f4f4", "#0b4fc9", "#c2500f"),
                Appearance::Light => ("#f1f0ec", "#3a6fd8", "#e2783a"),
            };
            self.ramp = Some(match self.kind.as_str() {
                "gradient" => Ramp(vec![c("low", "#cfe0f7")?, c("high", "#1d4f9e")?]),
                "gradient2" | "diverging" => {
                    mid = Some(self.trans.apply(p_num(p, "midpoint").unwrap_or(0.0)));
                    Ramp(vec![c("low", cold)?, c("mid", neutral)?, c("high", warm)?])
                }
                "gradientn" => {
                    let cs = p_strs(p, "colors")
                        .or_else(|| p_strs(p, "colours"))
                        .unwrap_or_default();
                    if cs.len() < 2 {
                        return bad("scale_*_gradientn: colors needs two or more colours");
                    }
                    Ramp(
                        cs.iter()
                            .map(|s| color_or_err(s))
                            .collect::<super::Result<Vec<_>>>()?,
                    )
                }
                _ => Ramp::viridis(
                    p_str(p, "option").unwrap_or("viridis"),
                    p_num(p, "begin").unwrap_or(0.0),
                    p_num(p, "end").unwrap_or(1.0),
                    p_num(p, "direction").unwrap_or(1.0) < 0.0,
                )?,
            });
        }
        // breaks: about five, inside the range
        let pos = {
            let mut s = PosScale::new('y');
            s.trans = self.trans;
            if let Some(Param::Str(f)) = self.params.get("labels")
                && let Some(n) = NumFmt::parse(f)
            {
                s.labels = Labels::Num(n);
            }
            if let Some(Param::Nums(b)) = self.params.get("breaks") {
                s.breaks = Breaks::At(b.clone());
            }
            s.expand = Some((0.0, 0.0));
            s
        };
        let t = pos.train(lo, hi, 230.0, false, None);
        self.map = Some(NpMap::Continuous {
            lo,
            hi,
            mid,
            breaks: t.breaks,
            labels: t.labels,
        });
        Ok(())
    }

    /// Where a continuous value (transformed space) falls, in [0, 1].
    pub fn unit(&self, v: f64) -> f64 {
        match &self.map {
            Some(NpMap::Continuous { lo, hi, mid, .. }) => {
                if let Some(m) = mid {
                    // centred: the mid at ½, the farther end at 0 or 1
                    let r = (hi - m).abs().max((m - lo).abs()).max(1e-12);
                    0.5 + (v - m) / r / 2.0
                } else if hi - lo > 1e-12 {
                    (v - lo) / (hi - lo)
                } else {
                    0.5
                }
            }
            _ => 0.0,
        }
    }

    /// A colour for a code (discrete) or a value (continuous).
    pub fn color_of(&self, code: Option<u32>, v: f64) -> Rgba {
        match (&self.map, code) {
            (Some(NpMap::Discrete { .. }), Some(c)) if c != super::data::NA => {
                self.palette[c as usize % self.palette.len().max(1)]
            }
            (Some(NpMap::Continuous { .. }), _) if v.is_finite() => self
                .ramp
                .as_ref()
                .map(|r| r.at(self.unit(v)))
                .unwrap_or(Rgba::new(128, 128, 128)),
            _ => Rgba::new(150, 150, 150),
        }
    }

    /// A size (ggplot units) for a code or value: area-proportional
    /// between `range` (default 1 to 6).
    pub fn size_of(&self, code: Option<u32>, v: f64) -> f64 {
        let r = super::p_nums(&self.params, "range").unwrap_or(vec![1.0, 6.0]);
        let (a, b) = (r[0], *r.get(1).unwrap_or(&r[0]));
        let t = match (&self.map, code) {
            (Some(NpMap::Discrete { levels, .. }), Some(c)) if c != super::data::NA => {
                if levels.len() <= 1 {
                    0.5
                } else {
                    c as f64 / (levels.len() - 1) as f64
                }
            }
            (Some(NpMap::Continuous { .. }), _) => self.unit(v),
            _ => return f64::NAN,
        };
        if self.kind == "radius" {
            a + (b - a) * t
        } else {
            (a * a + (b * b - a * a) * t.clamp(0.0, 1.0)).sqrt()
        }
    }

    pub fn alpha_of(&self, code: Option<u32>, v: f64) -> f64 {
        let r = super::p_nums(&self.params, "range").unwrap_or(vec![0.1, 1.0]);
        let (a, b) = (r[0], *r.get(1).unwrap_or(&r[0]));
        let t = match (&self.map, code) {
            (Some(NpMap::Discrete { levels, .. }), Some(c)) if c != super::data::NA => {
                if levels.len() <= 1 {
                    1.0
                } else {
                    c as f64 / (levels.len() - 1) as f64
                }
            }
            (Some(NpMap::Continuous { .. }), _) => self.unit(v),
            _ => return 1.0,
        };
        a + (b - a) * t.clamp(0.0, 1.0)
    }

    /// A line width (gg units, ×2.8 pixels) for a code or value: linear
    /// between `range` (default 0.25 to 2).
    pub fn linewidth_of(&self, code: Option<u32>, v: f64) -> f64 {
        let r = super::p_nums(&self.params, "range").unwrap_or(vec![0.25, 2.0]);
        let (a, b) = (r[0], *r.get(1).unwrap_or(&r[0]));
        let t = match (&self.map, code) {
            (Some(NpMap::Discrete { levels, .. }), Some(c)) if c != super::data::NA => {
                if levels.len() <= 1 {
                    0.5
                } else {
                    c as f64 / (levels.len() - 1) as f64
                }
            }
            (Some(NpMap::Continuous { .. }), _) if v.is_finite() => self.unit(v),
            _ => return f64::NAN,
        };
        a + (b - a) * t.clamp(0.0, 1.0)
    }

    pub fn shape_of(&self, code: Option<u32>) -> Shape {
        match code {
            Some(c) if c != super::data::NA => {
                if let Some(values) = p_strs(&self.params, "values")
                    && !values.is_empty()
                {
                    return Shape::parse(&values[c as usize % values.len()])
                        .unwrap_or(Shape::Circle);
                }
                Shape::ORDER[c as usize % Shape::ORDER.len()]
            }
            _ => Shape::Circle,
        }
    }

    pub fn dash_of(&self, code: Option<u32>) -> Option<Vec<f64>> {
        match code {
            Some(c) if c != super::data::NA => {
                if let Some(values) = p_strs(&self.params, "values")
                    && !values.is_empty()
                {
                    return dash(&values[c as usize % values.len()]);
                }
                dash(LINETYPES[c as usize % LINETYPES.len()])
            }
            _ => None,
        }
    }
}

pub const LINETYPES: [&str; 6] = [
    "solid", "dashed", "dotted", "dotdash", "longdash", "twodash",
];

/// A linetype's dash pattern (pixels), None for solid.
pub fn dash(name: &str) -> Option<Vec<f64>> {
    match name {
        "dashed" => Some(vec![5.0, 4.0]),
        "dotted" => Some(vec![1.5, 3.0]),
        "dotdash" => Some(vec![1.5, 3.0, 5.0, 3.0]),
        "longdash" => Some(vec![9.0, 4.0]),
        "twodash" => Some(vec![3.0, 3.0, 8.0, 3.0]),
        _ => None,
    }
}

fn quiet_n(app: Appearance, n: usize) -> super::Result<Vec<Rgba>> {
    let q = quiet(app);
    if n <= q.len() {
        return Ok(q[..n].iter().map(|h| Rgba::hex(h).unwrap()).collect());
    }
    // more levels than hues: viridis takes over (distinct, ordered)
    let ramp = Ramp::viridis("viridis", 0.0, 0.95, false)?;
    Ok((0..n).map(|i| ramp.at(i as f64 / (n - 1) as f64)).collect())
}

/// Natural order: numbers by value, text by its characters with digit
/// runs compared as numbers ("a2" before "a10").
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    if let (Ok(x), Ok(y)) = (a.parse::<f64>(), b.parse::<f64>()) {
        return x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal);
    }
    let (mut i, mut j) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (i.peek().copied(), j.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut x = String::new();
                while let Some(&c) = i.peek().filter(|c| c.is_ascii_digit()) {
                    x.push(c);
                    i.next();
                }
                let mut y = String::new();
                while let Some(&d) = j.peek().filter(|c| c.is_ascii_digit()) {
                    y.push(d);
                    j.next();
                }
                let (xv, yv) = (
                    x.parse::<u128>().unwrap_or(0),
                    y.parse::<u128>().unwrap_or(0),
                );
                if xv != yv {
                    return xv.cmp(&yv);
                }
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(&d);
                }
                i.next();
                j.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_breaks_are_nice() {
        assert_eq!(
            nice_breaks(0.0, 10.0, 5),
            vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]
        );
        assert_eq!(nice_breaks(0.13, 0.91, 4), vec![0.2, 0.4, 0.6, 0.8]);
        let b = nice_breaks(-3.2, 47.0, 5);
        assert_eq!(b, vec![0.0, 10.0, 20.0, 30.0, 40.0]);
    }

    #[test]
    fn log_breaks_are_decades() {
        let b = log_breaks(0.0, 5.0, 5);
        assert_eq!(b, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        let b: Vec<f64> = log_breaks(0.0, 1.0, 5)
            .iter()
            .map(|v| 10f64.powf(*v).round())
            .collect();
        assert_eq!(b, vec![1.0, 2.0, 5.0, 10.0]);
    }

    #[test]
    fn labels_by_format() {
        let v = [0.0, 0.25, 0.5];
        assert_eq!(
            format_numbers(&v, &NumFmt::Percent, 0.25, "", ""),
            vec!["0%", "25%", "50%"]
        );
        assert_eq!(
            format_numbers(&[1500.0, 250000.0], &NumFmt::Dollar, 1000.0, "", ""),
            vec!["$1,500", "$250,000"]
        );
        assert_eq!(
            format_numbers(&[0.0, 50000.0, 100000.0], &NumFmt::Auto, 50000.0, "", ""),
            vec!["0", "50,000", "100,000"]
        );
        assert_eq!(
            format_numbers(&[0.0, 2.5e6, 5e6], &NumFmt::Short, 2.5e6, "", ""),
            vec!["0", "2.5M", "5M"]
        );
        assert_eq!(
            format_numbers(&[0.5, 1.0, 1.5], &NumFmt::Auto, 0.5, "", ""),
            vec!["0.5", "1.0", "1.5"]
        );
    }

    #[test]
    fn time_breaks_follow_the_calendar() {
        let day = 86400.0;
        let lo = days_from_civil(2024, 1, 15) as f64 * day;
        let hi = days_from_civil(2024, 12, 20) as f64 * day;
        let (b, u) = time_breaks(lo, hi, 6, None);
        assert!(matches!(u, TimeUnit::Month(_)), "{u:?}");
        assert_eq!(strftime(b[0], "%Y-%m-%d"), "2024-04-01");
        let l = time_labels(&b, u, false);
        assert_eq!(l[0], "Apr");
        assert_eq!(
            strftime(0.0, "%a %e %B %Y %H:%M"),
            "Thu 1 January 1970 00:00"
        );
    }

    #[test]
    fn colours_and_ramps() {
        assert_eq!(color("steelblue").unwrap().rgb_hex(), "#4682b4");
        assert_eq!(color("grey50").unwrap().rgb_hex(), "#7f7f7f");
        assert!(color("nope").is_none());
        let r = Ramp::viridis("viridis", 0.0, 1.0, false).unwrap();
        assert_eq!(r.at(0.0).rgb_hex(), "#440154");
        assert_eq!(r.at(1.0).rgb_hex(), "#fde725");
        assert_eq!(natural_cmp("a2", "a10"), std::cmp::Ordering::Less);
        assert_eq!(natural_cmp("10", "9"), std::cmp::Ordering::Greater);
    }
}
