use logos::Logos;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Logos)]
#[logos(skip r"[ \t\r\n]+")]
#[logos(skip r"#[^\n]*")]
pub enum Token {
    #[token("sensor")]
    Sensor,

    #[token("output")]
    Output,

    #[token("on")]
    On,

    #[token("every")]
    Every,

    #[token("task")]
    Task,

    #[token("read")]
    Read,

    #[token("write")]
    Write,

    #[token("if")]
    If,

    #[token("else")]
    Else,

    #[token("sleep")]
    Sleep,

    #[token("while")]
    While,

    #[token("for")]
    For,

    #[token("unit")]
    UnitKw,

    #[token("val")]
    Val,

    #[token("extern")]
    Extern,

    #[token("fn")]
    Fn,

    #[token("return")]
    Return,

    #[token("void")]
    VoidKw,

    #[token("in")]
    In,

    #[token("using")]
    Using,

    #[token("true")]
    True,

    #[token("false")]
    False,

    #[token("->")]
    Arrow,

    #[token("<-")]
    LeftArrow,

    #[token(">=")]
    GreaterEq,

    #[token("<=")]
    LessEq,

    #[token("==")]
    Equals,

    #[token("!=")]
    NotEquals,

    #[token(">")]
    Greater,

    #[token("<")]
    Less,

    #[token("=")]
    Assign,

    #[token("{")]
    LBrace,

    #[token("}")]
    RBrace,

    #[token("[")]
    LBracket,

    #[token("]")]
    RBracket,

    #[token("(")]
    LParen,

    #[token(")")]
    RParen,

    #[token(",")]
    Comma,

    #[token(":")]
    Colon,

    #[token("::")]
    DoubleColon,

    #[token("..")]
    DotDot,

    #[token(";")]
    Semicolon,

    #[token("+")]
    Plus,

    #[token("-")]
    Minus,

    #[token("*")]
    Star,

    #[token("/")]
    Slash,

    #[token("%")]
    Percent,

    #[token("^")]
    Caret,

    #[token("&")]
    Ampersand,

    #[token("||")]
    Or,

    #[token("&&")]
    And,

    #[token("!")]
    Not,

    #[token(".")]
    Dot,

    #[regex(r"[0-9]+(\.[0-9]+)?[a-zA-Z]+", |lex| lex.slice().to_owned())]
    UnitLiteral(String),

    #[regex(r"[0-9]+", |lex| lex.slice().parse::<i64>().unwrap_or(0))]
    Int(i64),

    #[regex(r"[0-9]+\.[0-9]+", |lex| lex.slice().parse::<f64>().unwrap_or(0.0))]
    Float(f64),

    #[regex(r#""([^"\\]|\\.)*""#, |lex| {
        let s = lex.slice();

        let inner = &s[1..s.len()-1];
        inner.replace("\\n", "\n")
             .replace("\\t", "\t")
             .replace("\\r", "\r")
             .replace("\\\\", "\\")
             .replace("\\\"", "\"")
    })]
    String(String),

    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_owned())]
    Identifier(String),
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Sensor => write!(f, "sensor"),
            Token::Output => write!(f, "output"),
            Token::On => write!(f, "on"),
            Token::Every => write!(f, "every"),
            Token::Task => write!(f, "task"),
            Token::Read => write!(f, "read"),
            Token::Write => write!(f, "write"),
            Token::If => write!(f, "if"),
            Token::Else => write!(f, "else"),
            Token::Sleep => write!(f, "sleep"),
            Token::While => write!(f, "while"),
            Token::For => write!(f, "for"),
            Token::UnitKw => write!(f, "unit"),
            Token::Val => write!(f, "val"),
            Token::Extern => write!(f, "extern"),
            Token::Fn => write!(f, "fn"),
            Token::Return => write!(f, "return"),
            Token::VoidKw => write!(f, "void"),
            Token::In => write!(f, "in"),
            Token::Using => write!(f, "using"),
            Token::True => write!(f, "true"),
            Token::False => write!(f, "false"),
            Token::Arrow => write!(f, "->"),
            Token::LeftArrow => write!(f, "<-"),
            Token::Greater => write!(f, ">"),
            Token::Less => write!(f, "<"),
            Token::GreaterEq => write!(f, ">="),
            Token::LessEq => write!(f, "<="),
            Token::Equals => write!(f, "=="),
            Token::NotEquals => write!(f, "!="),
            Token::Assign => write!(f, "="),
            Token::Ampersand => write!(f, "&"),
            Token::And => write!(f, "&&"),
            Token::Or => write!(f, "||"),
            Token::Not => write!(f, "!"),
            Token::Dot => write!(f, "."),
            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Star => write!(f, "*"),
            Token::Slash => write!(f, "/"),
            Token::Percent => write!(f, "%"),
            Token::Caret => write!(f, "^"),
            Token::Int(i) => write!(f, "{}", i),
            Token::Float(flt) => write!(f, "{}", flt),
            Token::String(s) => write!(f, "\"{}\"", s),
            Token::Identifier(id) => write!(f, "{}", id),
            Token::UnitLiteral(unit) => write!(f, "{}", unit),
            Token::LBrace => write!(f, "{{"),
            Token::RBrace => write!(f, "}}"),
            Token::LBracket => write!(f, "["),
            Token::RBracket => write!(f, "]"),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::Comma => write!(f, ","),
            Token::Colon => write!(f, ":"),
            Token::DoubleColon => write!(f, "::"),
            Token::DotDot => write!(f, ".."),
            Token::Semicolon => write!(f, ";"),
        }
    }
}
