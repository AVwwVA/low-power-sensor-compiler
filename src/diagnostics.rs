use chumsky::{
    error::{Rich, RichReason},
    span::SimpleSpan,
};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

impl SourceSpan {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

impl From<SimpleSpan<usize>> for SourceSpan {
    fn from(span: SimpleSpan<usize>) -> Self {
        Self::new(span.start, span.end)
    }
}

pub struct SourceFile<'a> {
    path: &'a str,
    source: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> SourceFile<'a> {
    pub fn new(path: &'a str, source: &'a str) -> Self {
        let mut line_starts = vec![0];
        for (idx, ch) in source.char_indices() {
            if ch == '\n' {
                line_starts.push(idx + 1);
            }
        }

        Self {
            path,
            source,
            line_starts,
        }
    }

    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.source.len());
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let line_start = self.line_starts[line_idx];
        (
            line_idx + 1,
            self.source[line_start..offset].chars().count() + 1,
        )
    }

    fn line_start(&self, line_idx: usize) -> usize {
        self.line_starts[line_idx]
    }

    fn line_end(&self, line_idx: usize) -> usize {
        self.line_starts
            .get(line_idx + 1)
            .copied()
            .unwrap_or(self.source.len())
    }

    fn line_text(&self, line_idx: usize) -> &'a str {
        let start = self.line_start(line_idx);
        let end = self.line_end(line_idx);
        self.source[start..end].trim_end_matches('\n')
    }

    pub fn format_diagnostic(
        &self,
        stage: &str,
        message: &str,
        span: Option<SourceSpan>,
        label: Option<&str>,
    ) -> String {
        match span {
            Some(span) => self.format_spanned(stage, message, span, label),
            None => format!("{stage}: {message}"),
        }
    }

    fn format_spanned(
        &self,
        stage: &str,
        message: &str,
        span: SourceSpan,
        label: Option<&str>,
    ) -> String {
        let start = span.start.min(self.source.len());
        let end = span.end.max(start).min(self.source.len());
        let (line, col) = self.line_col(start);
        let line_idx = line - 1;
        let line_text = self.line_text(line_idx);
        let line_start = self.line_start(line_idx);
        let highlight_start = self.source[line_start..start].chars().count();
        let highlight_end = if end > start {
            self.source[line_start..end].chars().count()
        } else {
            highlight_start + 1
        };
        let caret_count = highlight_end.saturating_sub(highlight_start).max(1);
        let gutter = line.to_string().len().max(1);
        let label = label.unwrap_or(message);

        format!(
            "{stage}: {message}\n --> {}:{}:{}\n{} |\n{:>width$} | {}\n{} | {}{}",
            self.path,
            line,
            col,
            "",
            line,
            line_text,
            "",
            " ".repeat(highlight_start),
            "^".repeat(caret_count),
            width = gutter,
        ) + &format!(" {label}")
    }
}

pub fn concise_parse_error_message<T, S>(err: &Rich<'_, T, S>) -> String
where
    T: fmt::Display,
{
    match err.reason() {
        RichReason::ExpectedFound { .. } => match err.found() {
            Some(token) => format!("Unexpected token '{token}'"),
            None => "Unexpected end of input".to_string(),
        },
        RichReason::Custom(message) => message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        lexer::Lexer,
        parser::{program_parser, token_stream},
    };
    use chumsky::Parser;

    #[test]
    fn parse_error_message_omits_expected_token_suggestions() {
        let input = "every 1s { if (x > ) { } }";
        let lexer = Lexer::new();
        let tokens = lexer.tokenize_spanned(input).unwrap();
        let errs = program_parser()
            .parse(token_stream(&tokens, input.len()))
            .into_result()
            .unwrap_err();

        assert_eq!(
            concise_parse_error_message(&errs[0]),
            "Unexpected token ')'"
        );
    }

    #[test]
    fn parse_error_message_handles_unexpected_end_of_input() {
        let input = "every 1s {";
        let lexer = Lexer::new();
        let tokens = lexer.tokenize_spanned(input).unwrap();
        let errs = program_parser()
            .parse(token_stream(&tokens, input.len()))
            .into_result()
            .unwrap_err();

        assert_eq!(
            concise_parse_error_message(&errs[0]),
            "Unexpected end of input"
        );
    }
}
