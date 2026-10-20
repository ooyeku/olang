//! Number formatting by a spec: `str.fmt(x, ".2f")`, `ods.format(s,
//! ",.0f")`.
//!
//! The spec is Python's format mini-language, the part a report uses:
//!
//! ```text
//! [[fill]align][sign][$][,][width][.precision][type]
//! ```
//!
//! - `align` `<` `>` `^` with an optional fill character before it, and a
//!   `width` (the text is at least that wide);
//! - `sign`: `+` (always signed), `-` (the default), or ` ` (a space for
//!   a positive number);
//! - `$`: a currency symbol before the digits (after the sign: `-$1,234`);
//! - `,`: thousands separators;
//! - `.precision`: digits after the point (`f`, `%`, `e`), or significant
//!   digits (`g`);
//! - `type`: `f` fixed (the default with a precision), `%` a percentage
//!   (times 100, then `f`), `e` scientific, `g` general, `d` an integer
//!   (rounded).
//!
//! Rounding is the decimal expansion of the binary value, a half to even
//! on an exact tie (Python's and `str.fixed`'s). A rounded negative zero
//! prints without its sign, so a column never reads `-0.00`. NaN prints
//! `NaN` and the infinities `inf` / `-inf`.

#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    fill: char,
    align: Option<char>,
    sign: char,
    currency: Option<String>,
    comma: bool,
    width: usize,
    precision: Option<usize>,
    kind: char,
}

impl Spec {
    pub fn parse(spec: &str) -> Result<Spec, String> {
        let chars: Vec<char> = spec.chars().collect();
        let mut i = 0;
        let mut fill = ' ';
        let mut align = None;
        let is_align = |c: char| matches!(c, '<' | '>' | '^');
        if chars.len() >= 2 && is_align(chars[1]) {
            fill = chars[0];
            align = Some(chars[1]);
            i = 2;
        } else if !chars.is_empty() && is_align(chars[0]) {
            align = Some(chars[0]);
            i = 1;
        }
        let mut sign = '-';
        if i < chars.len() && matches!(chars[i], '+' | '-' | ' ') {
            sign = chars[i];
            i += 1;
        }
        let mut currency = None;
        if i < chars.len() && chars[i] == '$' {
            currency = Some("$".to_string());
            i += 1;
        }
        let mut comma = false;
        if i < chars.len() && chars[i] == ',' {
            comma = true;
            i += 1;
        }
        let mut width = 0usize;
        while i < chars.len() && chars[i].is_ascii_digit() {
            width = width * 10 + chars[i].to_digit(10).unwrap() as usize;
            i += 1;
        }
        // Python allows the comma after the width too.
        if i < chars.len() && chars[i] == ',' {
            comma = true;
            i += 1;
        }
        let mut precision = None;
        if i < chars.len() && chars[i] == '.' {
            i += 1;
            let start = i;
            let mut p = 0usize;
            while i < chars.len() && chars[i].is_ascii_digit() {
                p = p * 10 + chars[i].to_digit(10).unwrap() as usize;
                i += 1;
            }
            if i == start {
                return Err(format!(
                    "format spec \"{}\": a '.' needs digits after it",
                    spec
                ));
            }
            if p > 30 {
                return Err(format!("format spec \"{}\": at most 30 digits", spec));
            }
            precision = Some(p);
        }
        let mut kind = if precision.is_some() { 'f' } else { 'g' };
        if i < chars.len() && matches!(chars[i], 'f' | 'F' | '%' | 'e' | 'E' | 'g' | 'G' | 'd') {
            kind = chars[i].to_ascii_lowercase();
            if chars[i] == '%' {
                kind = '%';
            }
            i += 1;
        }
        if i != chars.len() {
            return Err(format!(
                "format spec \"{}\": unexpected \"{}\" (the spec is [[fill]align][sign][$][,][width][.precision][f|%|e|g|d])",
                spec,
                chars[i..].iter().collect::<String>()
            ));
        }
        if kind == 'd' && precision.is_some() {
            return Err(format!(
                "format spec \"{}\": an integer ('d') takes no precision",
                spec
            ));
        }
        Ok(Spec {
            fill,
            align,
            sign,
            currency,
            comma,
            width,
            precision,
            kind,
        })
    }

    /// A spec with a currency symbol of the caller's (`str.money`).
    pub fn money(symbol: &str, digits: usize) -> Spec {
        Spec {
            fill: ' ',
            align: None,
            sign: '-',
            currency: Some(symbol.to_string()),
            comma: true,
            width: 0,
            precision: Some(digits),
            kind: 'f',
        }
    }

    /// `x` as text.
    pub fn format(&self, x: f64) -> String {
        let mut out = String::new();
        self.format_into(x, &mut out);
        out
    }

    /// An integer as text: exact (no f64 round trip) for `d` and a plain
    /// `g`, as its f64 otherwise.
    pub fn format_i64(&self, n: i64, out: &mut String) {
        if self.kind != 'd' && !(self.kind == 'g' && self.precision.is_none()) {
            return self.format_into(n as f64, out);
        }
        let mut digits = n.unsigned_abs().to_string();
        if self.comma {
            digits = group(&digits);
        }
        let mut text = String::new();
        if n < 0 {
            text.push('-');
        } else if self.sign == '+' {
            text.push('+');
        } else if self.sign == ' ' {
            text.push(' ');
        }
        if let Some(c) = &self.currency {
            text.push_str(c);
        }
        text.push_str(&digits);
        self.pad(text, out)
    }

    /// A percentage with `digits` decimals (`str.pct`).
    pub fn percent(digits: usize) -> Spec {
        Spec {
            fill: ' ',
            align: None,
            sign: '-',
            currency: None,
            comma: false,
            width: 0,
            precision: Some(digits),
            kind: '%',
        }
    }

    /// `x` as text, appended to `out`.
    pub fn format_into(&self, x: f64, out: &mut String) {
        if x.is_nan() {
            return self.pad("NaN".to_string(), out);
        }
        let neg_input = x.is_sign_negative();
        let v = if self.kind == '%' { x * 100.0 } else { x };
        let abs = v.abs();
        let mut digits = if abs.is_infinite() {
            "inf".to_string()
        } else {
            match self.kind {
                'f' | '%' => format!("{:.*}", self.precision.unwrap_or(6), abs),
                'd' => format!("{:.0}", abs),
                'e' => sci(abs, self.precision.unwrap_or(6)),
                _ => general(abs, self.precision),
            }
        };
        if self.comma && !abs.is_infinite() {
            digits = group(&digits);
        }
        // A value that rounds to zero prints unsigned.
        let zero = !abs.is_infinite() && digits.bytes().all(|b| !(b'1'..=b'9').contains(&b));
        let neg = neg_input && !zero;
        let mut text = String::new();
        if neg {
            text.push('-');
        } else if self.sign == '+' {
            text.push('+');
        } else if self.sign == ' ' {
            text.push(' ');
        }
        if let Some(c) = &self.currency {
            text.push_str(c);
        }
        text.push_str(&digits);
        if self.kind == '%' {
            text.push('%');
        }
        self.pad(text, out)
    }

    fn pad(&self, text: String, out: &mut String) {
        let len = text.chars().count();
        if len >= self.width {
            out.push_str(&text);
            return;
        }
        let gap = self.width - len;
        let (left, right) = match self.align.unwrap_or('>') {
            '<' => (0, gap),
            '^' => (gap / 2, gap - gap / 2),
            _ => (gap, 0),
        };
        out.extend(std::iter::repeat_n(self.fill, left));
        out.push_str(&text);
        out.extend(std::iter::repeat_n(self.fill, right));
    }
}

/// Thousands separators in the integer part of a plain decimal.
fn group(s: &str) -> String {
    let (int, rest) = match s.find(['.', 'e']) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    };
    let n = int.len();
    let mut out = String::with_capacity(s.len() + n / 3);
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (n - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.push_str(rest);
    out
}

/// Scientific, Python's form: `1.50e+03`.
fn sci(x: f64, p: usize) -> String {
    let s = format!("{:.*e}", p, x);
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let e: i32 = exp.parse().unwrap_or(0);
    format!("{}e{}{:02}", mant, if e < 0 { '-' } else { '+' }, e.abs())
}

/// General: `p` significant digits (6 by default), fixed unless the
/// exponent is below −4 or at least `p`; trailing zeros dropped.
fn general(x: f64, p: Option<usize>) -> String {
    let p = p.unwrap_or(6).max(1);
    if x == 0.0 {
        return "0".to_string();
    }
    let s = format!("{:.*e}", p - 1, x);
    let e: i32 = s
        .split_once('e')
        .map(|(_, e)| e.parse().unwrap_or(0))
        .unwrap_or(0);
    let trim = |t: String| -> String {
        if t.contains('.') {
            t.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            t
        }
    };
    if e < -4 || e >= p as i32 {
        let (mant, _) = s.split_once('e').unwrap_or((&s, ""));
        let mant = trim(mant.to_string());
        format!("{}e{}{:02}", mant, if e < 0 { '-' } else { '+' }, e.abs())
    } else {
        let decimals = (p as i32 - 1 - e).max(0) as usize;
        trim(format!("{:.*}", decimals, x))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(x: f64, spec: &str) -> String {
        Spec::parse(spec).unwrap().format(x)
    }

    #[test]
    fn pythons_answers() {
        assert_eq!(f(3.14159, ".2f"), "3.14");
        assert_eq!(f(1234567.891, ",.0f"), "1,234,568");
        assert_eq!(f(0.1234, ".1%"), "12.3%");
        assert_eq!(f(-1234.5, "$,.0f"), "-$1,234");
        assert_eq!(f(1234.5, "$,.2f"), "$1,234.50");
        assert_eq!(f(12345.678, "e"), "1.234568e+04");
        assert_eq!(f(0.000012345, ".3g"), "1.23e-05");
        assert_eq!(f(1234.5, "g"), "1234.5");
        assert_eq!(f(-0.001, ".2f"), "0.00");
        assert_eq!(f(2.5, ".0f"), "2");
        assert_eq!(f(2.675, ".2f"), "2.67");
        assert_eq!(f(42.0, ">8.1f"), "    42.0");
        assert_eq!(f(42.0, "*<6d"), "42****");
        assert_eq!(f(5.0, "+.1f"), "+5.0");
        assert_eq!(f(f64::NAN, ".2f"), "NaN");
        assert_eq!(f(-f64::INFINITY, ",.2f"), "-inf");
        assert_eq!(f(1234567.0, "d"), "1234567");
        assert_eq!(f(1234567.0, ",d"), "1,234,567");
        let mut out = String::new();
        Spec::parse(",d")
            .unwrap()
            .format_i64(-12_345_678_901_234_567, &mut out);
        assert_eq!(out, "-12,345,678,901,234,567");
    }

    #[test]
    fn bad_specs_say_why() {
        assert!(Spec::parse(".f").is_err());
        assert!(Spec::parse(".2x").is_err());
        assert!(Spec::parse(".2d").is_err());
    }
}
