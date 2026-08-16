//! Rendering a Frame as a terminal table.
//!
//! A Frame's `display()` is the first thing anyone sees at the REPL, so it
//! is worth more than a shape and a type list: a table shows the data, and
//! seeing the data is most of what an exploratory session is for.
//!
//! Every limit here exists because the value being printed may be much
//! larger than the terminal. Rows, columns, total width, and individual
//! cells are each capped, and every cap that bites is reported in the
//! footer — a table that silently dropped a column would be worse than the
//! one-line summary it replaced.

use olang_ods::{DType, Frame, Scalar};

/// Rows shown before the table elides its middle. Half appear at the top
/// and half at the bottom, which is what makes a sorted column readable.
const MAX_ROWS: usize = 20;
/// Cells wider than this are truncated with an ellipsis.
const MAX_CELL: usize = 28;
/// Budget for the printed width, in characters. Columns that would push
/// the table past it are dropped and counted in the footer.
const MAX_WIDTH: usize = 100;

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Right,
}

impl Align {
    fn of(dtype: DType) -> Self {
        // Numbers line up on their last digit, text on its first letter.
        match dtype {
            DType::F64 | DType::I64 => Align::Right,
            DType::Bool | DType::Str => Align::Left,
        }
    }

    fn pad(self, text: &str, width: usize) -> String {
        let gap = width.saturating_sub(display_width(text));
        match self {
            Align::Left => format!("{}{}", text, " ".repeat(gap)),
            Align::Right => format!("{}{}", " ".repeat(gap), text),
        }
    }
}

/// Character count, which is what the box-drawing arithmetic needs — not
/// `len()`, which counts UTF-8 bytes and would misalign any non-ASCII cell.
fn display_width(text: &str) -> usize {
    text.chars().count()
}

fn truncate(text: &str) -> String {
    if display_width(text) <= MAX_CELL {
        return text.to_string();
    }
    let kept: String = text.chars().take(MAX_CELL - 1).collect();
    format!("{}…", kept)
}

fn cell(scalar: Scalar) -> String {
    truncate(&match scalar {
        // Floats go through the language's own formatter so a Frame and a
        // `println` of the same number never disagree.
        Scalar::F64(x) => crate::ast::format_float(x),
        Scalar::I64(x) => x.to_string(),
        Scalar::Bool(b) => b.to_string(),
        Scalar::Str(s) => s.to_string(),
        Scalar::Null => "—".to_string(),
    })
}

/// Which row indices to print, and where the elision falls. Returns the
/// indices in order; `None` marks the elided middle.
fn row_plan(n_rows: usize) -> Vec<Option<usize>> {
    if n_rows <= MAX_ROWS {
        return (0..n_rows).map(Some).collect();
    }
    let half = MAX_ROWS / 2;
    let mut plan: Vec<Option<usize>> = (0..half).map(Some).collect();
    plan.push(None);
    plan.extend((n_rows - half..n_rows).map(Some));
    plan
}

/// The Frame as a box-drawn table, header line included.
pub fn render(frame: &Frame) -> String {
    let n_rows = frame.n_rows();
    let n_cols = frame.n_cols();
    let shape = format!("Frame[{} x {}]", n_rows, n_cols);
    if n_cols == 0 {
        return format!("{} (no columns)", shape);
    }

    let plan = row_plan(n_rows);

    // Build every column that might be shown, then decide how many fit.
    let mut columns: Vec<(Align, Vec<String>)> = Vec::with_capacity(n_cols);
    for (name, series) in frame.names().iter().zip(frame.columns()) {
        let align = Align::of(series.dtype());
        let mut body = vec![truncate(name), series.dtype().to_string()];
        for entry in &plan {
            body.push(match entry {
                Some(i) => cell(series.scalar_at(*i)),
                None => "…".to_string(),
            });
        }
        columns.push((align, body));
    }

    let widths: Vec<usize> = columns
        .iter()
        .map(|(_, body)| body.iter().map(|c| display_width(c)).max().unwrap_or(0))
        .collect();

    // Each column costs its width plus the "│ " and " " around it; the
    // closing "│" is one more. Keep at least one column whatever happens,
    // so a single very wide column still prints rather than vanishing.
    let mut shown = 0;
    let mut used = 1;
    for width in &widths {
        let cost = width + 3;
        if shown > 0 && used + cost > MAX_WIDTH {
            break;
        }
        used += cost;
        shown += 1;
    }

    let widths = &widths[..shown];
    let rule = |left: &str, mid: &str, right: &str| {
        let bars: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
        format!("{}{}{}", left, bars.join(mid), right)
    };
    let line = |row: usize| {
        let cells: Vec<String> = columns[..shown]
            .iter()
            .zip(widths)
            .map(|((align, body), w)| format!(" {} ", align.pad(&body[row], *w)))
            .collect();
        format!("│{}│", cells.join("│"))
    };

    let mut out = vec![shape];
    out.push(rule("┌", "┬", "┐"));
    out.push(line(0)); // names
    out.push(line(1)); // dtypes
    out.push(rule("├", "┼", "┤"));
    if plan.is_empty() {
        let inner = widths.iter().sum::<usize>() + 3 * widths.len() - 1;
        let label = "(no rows)";
        let gap = inner.saturating_sub(display_width(label) + 1);
        out.push(format!("│ {}{}│", label, " ".repeat(gap)));
    } else {
        for row in 0..plan.len() {
            out.push(line(row + 2));
        }
    }
    out.push(rule("└", "┴", "┘"));

    // Anything the caps hid is stated rather than left to be discovered.
    let mut notes = Vec::new();
    let shown_rows = plan.iter().filter(|entry| entry.is_some()).count();
    if n_rows > shown_rows {
        notes.push(format!("{} rows not shown", n_rows - shown_rows));
    }
    if shown < n_cols {
        notes.push(format!("{} columns not shown", n_cols - shown));
    }
    if !notes.is_empty() {
        out.push(format!("… {}", notes.join(", ")));
    }
    out.join("\n")
}
