//! Number formatting by a spec: `str.fmt(x, ".2f")`, `ods.format(s,
//! ",.0f")`.
//!
//! The spec is Python's format mini-language:
//!
//! ```text
//! [[fill]align][sign][z][#][$][0][width][,|_][.precision][type]
//! ```
//!
//! - `align` `<` `>` `^` `=` with an optional fill character before it,
//!   and a `width` (the text is at least that wide); `=` pads between the
//!   sign and the digits (`+0005`);
//! - `sign`: `+` (always signed), `-` (the default), or ` ` (a space for
//!   a positive number);
//! - `z`: accepted for Python's sake; a value that rounds to zero always
//!   prints unsigned here;
//! - `#`: the alternate form: `0x` `0o` `0b` before a hexadecimal, octal
//!   or binary number, a point kept by `.0f`, trailing zeros kept by `g`;
//! - `$`: a currency symbol before the digits (after the sign: `-$1,234`);
//!   not Python's;
//! - `0`: zero padding, the fill `0` and the alignment `=` (`02d` of 5 is
//!   `05`); with a separator the zeros are grouped too (`08,d` of 1234 is
//!   `0,001,234`);
//! - `,` or `_`: thousands separators (`_` groups a hexadecimal, octal or
//!   binary number by four digits);
//! - `.precision`: digits after the point (`f`, `%`, `e`), or significant
//!   digits (`g`);
//! - `type`: `f` fixed (the default with a precision), `%` a percentage
//!   (times 100, then `f`), `e` scientific, `g` general, `n` a number (`d`
//!   for an integer, `g` otherwise), `d` an integer (rounded), `x` `o` `b`
//!   an integer in hexadecimal, octal or binary; `F` `E` `G` `X` write
//!   their letters in upper case.
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
    alternate: bool,
    currency: Option<String>,
    /// The thousands separator, if any.
    sep: Option<char>,
    width: usize,
    precision: Option<usize>,
    kind: char,
    /// `F` `E` `G` `X`: letters in upper case.
    upper: bool,
}

const GRAMMAR: &str = "[[fill]align][sign][z][#][$][0][width][,|_][.precision][f|%|e|g|n|d|x|o|b]";

impl Spec {
    pub fn parse(spec: &str) -> Result<Spec, String> {
        let chars: Vec<char> = spec.chars().collect();
        let mut i = 0;
        let mut fill = None;
        let mut align = None;
        let is_align = |c: char| matches!(c, '<' | '>' | '^' | '=');
        if chars.len() >= 2 && is_align(chars[1]) {
            fill = Some(chars[0]);
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
        if i < chars.len() && chars[i] == 'z' {
            i += 1;
        }
        let mut alternate = false;
        if i < chars.len() && chars[i] == '#' {
            alternate = true;
            i += 1;
        }
        let mut currency = None;
        if i < chars.len() && chars[i] == '$' {
            currency = Some("$".to_string());
            i += 1;
        }
        let mut sep = None;
        if i < chars.len() && chars[i] == ',' {
            sep = Some(',');
            i += 1;
        }
        // The `0` flag: zeros fill between the sign and the digits.
        if i < chars.len() && chars[i] == '0' {
            fill = fill.or(Some('0'));
            align = align.or(Some('='));
            i += 1;
        }
        let mut width = 0usize;
        while i < chars.len() && chars[i].is_ascii_digit() {
            width = width
                .saturating_mul(10)
                .saturating_add(chars[i].to_digit(10).unwrap() as usize);
            i += 1;
        }
        if width > 1000 {
            return Err(format!("format spec \"{}\": a width of at most 1000", spec));
        }
        // Python's separator comes after the width.
        if i < chars.len() && matches!(chars[i], ',' | '_') {
            sep = Some(chars[i]);
            i += 1;
        }
        let mut precision = None;
        if i < chars.len() && chars[i] == '.' {
            i += 1;
            let start = i;
            let mut p = 0usize;
            while i < chars.len() && chars[i].is_ascii_digit() {
                p = p
                    .saturating_mul(10)
                    .saturating_add(chars[i].to_digit(10).unwrap() as usize);
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
        let mut upper = false;
        if i < chars.len()
            && matches!(
                chars[i],
                'f' | 'F' | '%' | 'e' | 'E' | 'g' | 'G' | 'n' | 'd' | 'x' | 'X' | 'o' | 'b'
            )
        {
            upper = matches!(chars[i], 'F' | 'E' | 'G' | 'X');
            kind = chars[i].to_ascii_lowercase();
            i += 1;
        }
        if i != chars.len() {
            return Err(format!(
                "format spec \"{}\": unexpected \"{}\" (the spec is {})",
                spec,
                chars[i..].iter().collect::<String>(),
                GRAMMAR
            ));
        }
        let integer = matches!(kind, 'd' | 'x' | 'o' | 'b');
        if integer && precision.is_some() {
            return Err(format!(
                "format spec \"{}\": an integer ('{}') takes no precision",
                spec, kind
            ));
        }
        if matches!(kind, 'x' | 'o' | 'b') && sep == Some(',') {
            return Err(format!(
                "format spec \"{}\": ',' does not group a '{}' number ('_' groups it by four)",
                spec, kind
            ));
        }
        if kind == 'n' && sep.is_some() {
            return Err(format!("format spec \"{}\": 'n' takes no separator", spec));
        }
        Ok(Spec {
            fill: fill.unwrap_or(' '),
            align,
            sign,
            alternate,
            currency,
            sep,
            width,
            precision,
            kind,
            upper,
        })
    }

    /// A spec with a currency symbol of the caller's (`str.money`).
    pub fn money(symbol: &str, digits: usize) -> Spec {
        Spec {
            fill: ' ',
            align: None,
            sign: '-',
            alternate: false,
            currency: Some(symbol.to_string()),
            sep: Some(','),
            width: 0,
            precision: Some(digits),
            kind: 'f',
            upper: false,
        }
    }

    /// A percentage with `digits` decimals (`str.pct`).
    pub fn percent(digits: usize) -> Spec {
        Spec {
            fill: ' ',
            align: None,
            sign: '-',
            alternate: false,
            currency: None,
            sep: None,
            width: 0,
            precision: Some(digits),
            kind: '%',
            upper: false,
        }
    }

    /// `x` as text.
    pub fn format(&self, x: f64) -> String {
        let mut out = String::new();
        self.format_into(x, &mut out);
        out
    }

    /// The radix of an integer kind.
    fn radix(&self) -> Option<u32> {
        match self.kind {
            'x' => Some(16),
            'o' => Some(8),
            'b' => Some(2),
            _ => None,
        }
    }

    /// An integer as text: exact (no f64 round trip) for the integer
    /// kinds, `n` and a plain `g`, as its f64 otherwise.
    pub fn format_i64(&self, n: i64, out: &mut String) {
        let exact = matches!(self.kind, 'd' | 'n' | 'x' | 'o' | 'b')
            || (self.kind == 'g' && self.precision.is_none());
        if !exact {
            return self.format_into(n as f64, out);
        }
        let digits = match self.radix() {
            Some(16) => format!("{:x}", n.unsigned_abs()),
            Some(8) => format!("{:o}", n.unsigned_abs()),
            Some(_) => format!("{:b}", n.unsigned_abs()),
            None => n.unsigned_abs().to_string(),
        };
        self.emit(n < 0, digits, true, out)
    }

    /// `x` as text, appended to `out`.
    pub fn format_into(&self, x: f64, out: &mut String) {
        if x.is_nan() {
            let t = if self.upper { "NAN" } else { "NaN" };
            return self.pad_plain(t.to_string(), out);
        }
        if self.radix().is_some() || (self.kind == 'n' && x.fract() == 0.0 && x.abs() < 9.0e15) {
            // An integer kind of a Float: its rounded value, as `d` does.
            let r = x.round_ties_even();
            if r.abs() < 9.2e18 {
                return self.format_i64(r as i64, out);
            }
        }
        let neg_input = x.is_sign_negative();
        let v = if self.kind == '%' { x * 100.0 } else { x };
        let abs = v.abs();
        let digits = if abs.is_infinite() {
            "inf".to_string()
        } else {
            match self.kind {
                'f' | '%' => {
                    let mut t = format!("{:.*}", self.precision.unwrap_or(6), abs);
                    if self.alternate && !t.contains('.') {
                        t.push('.');
                    }
                    t
                }
                'd' => format!("{:.0}", abs),
                'e' => sci(abs, self.precision.unwrap_or(6), self.alternate),
                _ => general(abs, self.precision, self.alternate),
            }
        };
        // A value that rounds to zero prints unsigned.
        let zero = !abs.is_infinite() && digits.bytes().all(|b| !(b'1'..=b'9').contains(&b));
        self.emit(neg_input && !zero, digits, abs.is_finite(), out)
    }

    /// Sign, currency, base prefix, the digits grouped and padded, and a
    /// percent sign.
    fn emit(&self, neg: bool, digits: String, finite: bool, out: &mut String) {
        let mut prefix = String::new();
        if neg {
            prefix.push('-');
        } else if self.sign == '+' {
            prefix.push('+');
        } else if self.sign == ' ' {
            prefix.push(' ');
        }
        if let Some(c) = &self.currency {
            prefix.push_str(c);
        }
        if self.alternate
            && let Some(r) = self.radix()
        {
            prefix.push_str(match r {
                16 => "0x",
                8 => "0o",
                _ => "0b",
            });
        }
        let suffix = if self.kind == '%' { "%" } else { "" };
        let every = if self.radix().is_some() { 4 } else { 3 };
        let group = |d: &str| match self.sep {
            Some(sep) if finite => group_by(d, sep, every),
            _ => d.to_string(),
        };
        let mut body = group(&digits);
        let used = prefix.chars().count() + suffix.chars().count();
        if self.align == Some('=') {
            let want = self.width.saturating_sub(used);
            if self.fill == '0' && self.sep.is_some() && finite {
                // Zeros join the integer part, grouped like the digits.
                let mut raw = digits;
                while body.chars().count() < want {
                    raw.insert(0, '0');
                    body = group(&raw);
                }
            } else {
                let gap = want.saturating_sub(body.chars().count());
                body = std::iter::repeat_n(self.fill, gap).collect::<String>() + &body;
            }
        }
        let mut text = prefix + &body + suffix;
        if self.upper {
            text = text.to_uppercase();
        }
        self.pad_plain(text, out)
    }

    /// `text` aligned in the width (`=` has padded it already).
    fn pad_plain(&self, text: String, out: &mut String) {
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

/// Separators every `every` digits in the integer part of a plain
/// number.
fn group_by(s: &str, sep: char, every: usize) -> String {
    let (int, rest) = match s.find(['.', 'e']) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    };
    let n = int.len();
    let mut out = String::with_capacity(s.len() + n / every);
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (n - i) % every == 0 {
            out.push(sep);
        }
        out.push(ch);
    }
    out.push_str(rest);
    out
}

/// Scientific, Python's form: `1.50e+03` (`#` keeps the point of `.0e`).
fn sci(x: f64, p: usize, alternate: bool) -> String {
    let s = format!("{:.*e}", p, x);
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let e: i32 = exp.parse().unwrap_or(0);
    let point = if alternate && !mant.contains('.') {
        "."
    } else {
        ""
    };
    format!(
        "{}{}e{}{:02}",
        mant,
        point,
        if e < 0 { '-' } else { '+' },
        e.abs()
    )
}

/// General: `p` significant digits (6 by default), fixed unless the
/// exponent is below −4 or at least `p`; trailing zeros dropped (kept by
/// `#`).
fn general(x: f64, p: Option<usize>, alternate: bool) -> String {
    let p = p.unwrap_or(6).max(1);
    if x == 0.0 {
        return if alternate {
            format!("{:.*}", p - 1, 0.0)
        } else {
            "0".to_string()
        };
    }
    let s = format!("{:.*e}", p - 1, x);
    let e: i32 = s
        .split_once('e')
        .map(|(_, e)| e.parse().unwrap_or(0))
        .unwrap_or(0);
    let trim = |t: String| -> String {
        if alternate {
            if t.contains('.') { t } else { t + "." }
        } else if t.contains('.') {
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
#[allow(clippy::approx_constant)] // 3.14159 here is a number to format, not π
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
    fn pythons_flags() {
        let i = |n: i64, spec: &str| {
            let mut out = String::new();
            Spec::parse(spec).unwrap().format_i64(n, &mut out);
            out
        };
        assert_eq!(i(5, "02d"), "05");
        assert_eq!(i(-5, "03d"), "-05");
        assert_eq!(i(5, "+05d"), "+0005");
        assert_eq!(i(1234, "08,d"), "0,001,234");
        assert_eq!(i(1234, "09,d"), "0,001,234");
        assert_eq!(i(1234567, "_d"), "1_234_567");
        assert_eq!(i(255, "x"), "ff");
        assert_eq!(i(255, "#X"), "0XFF");
        assert_eq!(i(255, "#06x"), "0x00ff");
        assert_eq!(i(8, "o"), "10");
        assert_eq!(i(5, "#b"), "0b101");
        assert_eq!(i(65535, "_x"), "ffff");
        assert_eq!(i(65536, "_x"), "1_0000");
        assert_eq!(i(5, "*=+6d"), "+****5");
        assert_eq!(i(5, "<05d"), "50000");
        assert_eq!(i(42, "n"), "42");
        assert_eq!(f(3.14159, "08.3f"), "0003.142");
        assert_eq!(f(-3.5, "=+8.1f"), "-    3.5");
        assert_eq!(f(1234.5, "010,.1f"), "0,001,234.5");
        assert_eq!(f(2.0, "#.0f"), "2.");
        assert_eq!(f(2.0, "#g"), "2.00000");
        assert_eq!(f(12345.678, "E"), "1.234568E+04");
        assert_eq!(f(f64::INFINITY, "F"), "INF");
        assert_eq!(f(0.5, "z.1f"), "0.5");
        assert_eq!(f(1234.5, "n"), "1234.5");
        assert_eq!(f(0.25, "05.0%"), "0025%");
        assert_eq!(f(255.0, "x"), "ff");
    }

    #[test]
    fn bad_specs_say_why() {
        assert!(Spec::parse(".f").is_err());
        assert!(Spec::parse(".2x").is_err());
        assert!(Spec::parse(".2d").is_err());
        assert!(Spec::parse(",x").is_err());
        assert!(Spec::parse(".1b").is_err());
    }
}
