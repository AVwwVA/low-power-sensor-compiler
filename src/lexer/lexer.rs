use crate::diagnostics::SourceSpan;
use crate::lexer::tokens::Token;
use chumsky::span::SimpleSpan;
use logos::Logos;
use std::fmt;

pub type SpannedToken = (Token, SimpleSpan<usize>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub message: String,
    pub span: SourceSpan,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {}..{}",
            self.message, self.span.start, self.span.end
        )
    }
}

pub struct Lexer;

impl Lexer {
    pub fn new() -> Self {
        Self
    }

    #[cfg(test)]
    pub fn tokenize(&self, input: &str) -> Result<Vec<Token>, ()> {
        self.tokenize_spanned(input)
            .map(|tokens| tokens.into_iter().map(|(token, _)| token).collect())
            .map_err(|_| ())
    }

    pub fn tokenize_spanned(&self, input: &str) -> Result<Vec<SpannedToken>, Vec<LexError>> {
        let mut tokens = Vec::new();
        let mut errors = Vec::new();

        for (result, span) in Token::lexer(input).spanned() {
            match result {
                Ok(token) => tokens.push((token, span.into())),
                Err(()) => errors.push(LexError {
                    message: "Invalid token".to_string(),
                    span: SourceSpan::new(span.start, span.end),
                }),
            }
        }

        if errors.is_empty() {
            Ok(tokens)
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_tokens() {
        let lexer = Lexer::new();
        let input = "sensor on every task read write sleep if else while for unit val extern fn return void in using true false";
        let tokens = lexer.tokenize(input).unwrap();

        assert_eq!(
            tokens,
            vec![
                Token::Sensor,
                Token::On,
                Token::Every,
                Token::Task,
                Token::Read,
                Token::Write,
                Token::Sleep,
                Token::If,
                Token::Else,
                Token::While,
                Token::For,
                Token::UnitKw,
                Token::Val,
                Token::Extern,
                Token::Fn,
                Token::Return,
                Token::VoidKw,
                Token::In,
                Token::Using,
                Token::True,
                Token::False
            ]
        );
    }

    #[test]
    fn test_numbers() {
        let lexer = Lexer::new();
        let input = "3.5 15";
        let tokens = lexer.tokenize(input).unwrap();

        assert_eq!(tokens, vec![Token::Float(3.5), Token::Int(15)]);
    }

    #[test]
    fn test_units() {
        let lexer = Lexer::new();
        let input = "1s 500ms 2h";
        let tokens = lexer.tokenize(input).unwrap();

        assert_eq!(
            tokens,
            vec![
                Token::UnitLiteral("1s".to_string()),
                Token::UnitLiteral("500ms".to_string()),
                Token::UnitLiteral("2h".to_string())
            ]
        );
    }

    #[test]
    fn test_identifiers() {
        let lexer = Lexer::new();
        let input = "temp sensor1 _var123";
        let tokens = lexer.tokenize(input).unwrap();

        assert_eq!(
            tokens,
            vec![
                Token::Identifier("temp".to_string()),
                Token::Identifier("sensor1".to_string()),
                Token::Identifier("_var123".to_string())
            ]
        );
    }

    #[test]
    fn test_operators() {
        let lexer = Lexer::new();
        let input = "-> <- > < >= <= == != + - * / % ^ & ||";
        let tokens = lexer.tokenize(input).unwrap();

        assert_eq!(
            tokens,
            vec![
                Token::Arrow,
                Token::LeftArrow,
                Token::Greater,
                Token::Less,
                Token::GreaterEq,
                Token::LessEq,
                Token::Equals,
                Token::NotEquals,
                Token::Plus,
                Token::Minus,
                Token::Star,
                Token::Slash,
                Token::Percent,
                Token::Caret,
                Token::Ampersand,
                Token::Or
            ]
        );
    }

    #[test]
    fn test_delimiters() {
        let lexer = Lexer::new();
        let input = "( ) { } [ ] , ; ..";
        let tokens = lexer.tokenize(input).unwrap();

        assert_eq!(
            tokens,
            vec![
                Token::LParen,
                Token::RParen,
                Token::LBrace,
                Token::RBrace,
                Token::LBracket,
                Token::RBracket,
                Token::Comma,
                Token::Semicolon,
                Token::DotDot,
            ]
        );
    }

    #[test]
    fn test_range_literal_tokens() {
        let lexer = Lexer::new();
        let input = "[0..5]";
        let tokens = lexer.tokenize(input).unwrap();

        assert_eq!(
            tokens,
            vec![
                Token::LBracket,
                Token::Int(0),
                Token::DotDot,
                Token::Int(5),
                Token::RBracket,
            ]
        );
    }

    #[test]
    fn test_simple_dsl() {
        let lexer = Lexer::new();
        let input = "sensor temp on A0\n# This is a comment\nevery 1s {";
        let tokens = lexer.tokenize(input).unwrap();

        assert_eq!(
            tokens,
            vec![
                Token::Sensor,
                Token::Identifier("temp".to_string()),
                Token::On,
                Token::Identifier("A0".to_string()),
                Token::Every,
                Token::UnitLiteral("1s".to_string()),
                Token::LBrace,
            ]
        );
    }
    #[test]
    fn test_invalid_token() {
        let lexer = Lexer::new();
        let input = "sensor temp on A0 @";
        let result = lexer.tokenize(input);

        assert!(result.is_err());
    }
    #[test]
    fn test_empty_input() {
        let lexer = Lexer::new();
        let input = "";
        let tokens = lexer.tokenize(input).unwrap();

        assert!(tokens.is_empty());
    }

    #[test]
    fn test_whitespace() {
        let lexer = Lexer::new();
        let input = "   sensor   temp     on \n  A0  ";
        let tokens = lexer.tokenize(input).unwrap();

        assert_eq!(
            tokens,
            vec![
                Token::Sensor,
                Token::Identifier("temp".to_string()),
                Token::On,
                Token::Identifier("A0".to_string())
            ]
        );
    }
}
