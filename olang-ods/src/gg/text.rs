//! Text measurement without a font file: advance widths of a neo-grotesk
//! sans (Helvetica's metrics, per 1000 em), which the system UI faces the
//! chart is drawn in (SF, Segoe, Roboto) track within a few percent. The
//! layout leaves that much slack.

const ASCII: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // ' '..'/'
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // 0-9
    278, 278, 584, 584, 584, 556, 1015, // :..@
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667,
    611, 722, 667, 944, 667, 667, 611, // A-Z
    278, 278, 278, 469, 556, 333, // [ \ ] ^ _ `
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500,
    278, 556, 500, 722, 500, 500, 500, // a-z
    334, 260, 334, 584, // { | } ~
];

/// A text's width at `size` (pixels), `bold` a little wider.
pub fn width(s: &str, size: f64, bold: bool) -> f64 {
    let mut w = 0u32;
    for c in s.chars() {
        let cw = match c as u32 {
            0x20..=0x7e => ASCII[(c as u32 - 0x20) as usize] as u32,
            // digits and punctuation of other scripts, accents: average
            0x80..=0x2fff => 560,
            // CJK and wide symbols
            _ => 1000,
        };
        w += cw;
    }
    let k = if bold { 1.06 } else { 1.0 };
    // the system faces run a touch wider than Helvetica's metrics
    w as f64 * size / 1000.0 * k * 1.04
}

/// Cut a text to `max_w` with an ellipsis.
pub fn fit(s: &str, size: f64, bold: bool, max_w: f64) -> String {
    if width(s, size, bold) <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut n = chars.len();
    while n > 0 {
        n -= 1;
        let t: String = chars[..n].iter().collect::<String>() + "…";
        if width(&t, size, bold) <= max_w {
            return t;
        }
    }
    "…".to_string()
}

/// Wrap a text to lines no wider than `max_w`, at spaces.
pub fn wrap(s: &str, size: f64, bold: bool, max_w: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in s.split('\n') {
        let mut cur = String::new();
        for word in para.split(' ') {
            let trial = if cur.is_empty() {
                word.to_string()
            } else {
                format!("{} {}", cur, word)
            };
            if width(&trial, size, bold) > max_w && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            } else {
                cur = trial;
            }
        }
        lines.push(cur);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_and_fits() {
        let w = width("Hello", 10.0, false);
        assert!(w > 20.0 && w < 26.0, "{w}");
        assert!(width("W", 10.0, false) > width("i", 10.0, false) * 3.0);
        let cut = fit("a long category name", 11.0, false, 50.0);
        assert!(cut.ends_with('…') && width(&cut, 11.0, false) <= 50.0);
        assert_eq!(wrap("one two three", 10.0, false, 30.0).len(), 3);
    }
}
