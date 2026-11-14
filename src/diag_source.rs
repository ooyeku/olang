//! The source a printed diagnostic quotes: miette's `NamedSource`, with the
//! column of a report's `file:line:col` header counted in characters.
//!
//! miette counts that column in bytes, so a line with `é` or `🎉` before the
//! error put the header columns to the right of it, while `olang check
//! --format json` and the language server count characters (OL-371).

use miette::{MietteError, MietteSpanContents, SourceCode, SourceSpan, SpanContents};

/// A named source text whose span columns are 1 character each.
#[derive(Debug, Clone)]
pub struct CharSource {
    name: String,
    text: String,
}

impl CharSource {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> CharSource {
        CharSource {
            name: name.into(),
            text: text.into(),
        }
    }
}

impl SourceCode for CharSource {
    fn read_span<'a>(
        &'a self,
        span: &SourceSpan,
        context_lines_before: usize,
        context_lines_after: usize,
    ) -> Result<Box<dyn SpanContents<'a> + 'a>, MietteError> {
        let inner = self
            .text
            .read_span(span, context_lines_before, context_lines_after)?;
        // miette gives a column only when it reads no lines before the span
        // (the header's read); it is the bytes from the line's start.
        let column = if context_lines_before == 0 && inner.column() > 0 {
            let at = span.offset().min(self.text.len());
            let line_start = self.text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
            self.text
                .get(line_start..at)
                .map(|s| s.chars().count())
                .unwrap_or(inner.column())
        } else {
            inner.column()
        };
        Ok(Box::new(MietteSpanContents::new_named(
            self.name.clone(),
            inner.data(),
            *inner.span(),
            inner.line(),
            column,
            inner.line_count(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_column_counts_characters() {
        let text = "let x = 1\nprintln(\"héllo wörld 🎉\"); let y = undefined_thing + 1\n";
        let at = text.find("undefined_thing").unwrap();
        let diagnostic = miette::MietteDiagnostic::new("Undefined variable")
            .with_label(miette::LabeledSpan::at_offset(at, "here"));
        let report =
            miette::Report::new(diagnostic).with_source_code(CharSource::new("n1.ol", text));
        let shown = format!("{:?}", report);
        // `undefined_thing` is character 35 of line 2 (byte 40)
        assert!(shown.contains("n1.ol:2:35"), "{shown}");
    }
}
