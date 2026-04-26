use crate::ast::*;
use crate::lexer::SpannedToken;
use crate::lexer::tokens::Token;
use crate::types::Type;
use chumsky::error::Rich;
use chumsky::input::{Stream, ValueInput};
use chumsky::pratt::{infix, left, prefix, right};
use chumsky::prelude::*;
use chumsky::span::SimpleSpan;

type ParseExtra<'a> = extra::Err<Rich<'a, Token, SimpleSpan<usize>>>;

pub fn token_stream<'a>(
    tokens: &'a [SpannedToken],
    source_len: usize,
) -> impl ValueInput<'a, Token = Token, Span = SimpleSpan<usize>> + 'a {
    Stream::from_iter(tokens.iter().cloned()).map((source_len..source_len).into(), |(t, s)| (t, s))
}

fn spanned_expr<'a, I, P>(parser: P) -> impl Parser<'a, I, Expr, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
    P: Parser<'a, I, Expr, ParseExtra<'a>> + Clone,
{
    parser.map_with(|expr, e| expr.with_span(e.span().into()))
}

fn spanned_stmt<'a, I, P>(parser: P) -> impl Parser<'a, I, Statement, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
    P: Parser<'a, I, Statement, ParseExtra<'a>> + Clone,
{
    parser.map_with(|stmt, e| stmt.with_span(e.span().into()))
}

fn spanned_top_level<'a, I, P>(parser: P) -> impl Parser<'a, I, TopLevel, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
    P: Parser<'a, I, TopLevel, ParseExtra<'a>> + Clone,
{
    parser.map_with(|top, e| top.with_span(e.span().into()))
}

fn ident<'a, I>() -> impl Parser<'a, I, String, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    select! { Token::Identifier(s) => s }
}

fn path_segment<'a, I>() -> impl Parser<'a, I, String, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    choice((
        ident(),
        just(Token::Sensor).to("sensor".to_string()),
        just(Token::Output).to("output".to_string()),
        just(Token::On).to("on".to_string()),
        just(Token::Every).to("every".to_string()),
        just(Token::Task).to("task".to_string()),
        just(Token::Read).to("read".to_string()),
        just(Token::Write).to("write".to_string()),
        just(Token::Sleep).to("sleep".to_string()),
        just(Token::UnitKw).to("unit".to_string()),
        just(Token::Val).to("val".to_string()),
        just(Token::Fn).to("fn".to_string()),
        just(Token::In).to("in".to_string()),
        just(Token::Using).to("using".to_string()),
    ))
}

fn namespaced_ident<'a, I>() -> impl Parser<'a, I, Vec<String>, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    ident()
        .then(
            just(Token::DoubleColon)
                .ignore_then(path_segment())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(head, tail)| {
            let mut path = vec![head];
            path.extend(tail);
            path
        })
}

fn int_lit<'a, I>() -> impl Parser<'a, I, Expr, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    spanned_expr(select! { Token::Int(n) => Expr::new(ExprKind::IntLit(n)) })
}

fn signed_int<'a, I>() -> impl Parser<'a, I, i64, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    choice((
        just(Token::Minus)
            .ignore_then(select! { Token::Int(n) => n })
            .map(|n| -n),
        select! { Token::Int(n) => n },
    ))
}

fn float_lit<'a, I>() -> impl Parser<'a, I, Expr, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    spanned_expr(select! { Token::Float(f) => Expr::new(ExprKind::FloatLit(f)) })
}

fn bool_lit<'a, I>() -> impl Parser<'a, I, Expr, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    spanned_expr(select! {
        Token::True => Expr::new(ExprKind::BoolLit(true)),
        Token::False => Expr::new(ExprKind::BoolLit(false)),
    })
}

fn string_lit<'a, I>() -> impl Parser<'a, I, Expr, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    spanned_expr(select! { Token::String(s) => Expr::new(ExprKind::StringLit(s)) })
}

fn parse_unit_str(s: &str) -> Option<(Number, String)> {
    let num_end = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let (num_str, unit_str) = s.split_at(num_end);

    if let Ok(i) = num_str.parse::<i64>() {
        return Some((Number::Int(i), unit_str.to_string()));
    }
    if let Ok(f) = num_str.parse::<f64>() {
        return Some((Number::Float(f), unit_str.to_string()));
    }
    None
}

fn unit_lit<'a, I>() -> impl Parser<'a, I, Expr, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    spanned_expr(select! {
        Token::UnitLiteral(u) => {
            parse_unit_str(&u)
                .map(|(value, unit)| Expr::new(ExprKind::UnitLit { value, unit }))
                .unwrap_or(Expr::new(ExprKind::UnitLit { value: Number::Int(0), unit: u }))
        }
    })
}

fn explicit_cast_target<'a, I>() -> impl Parser<'a, I, Type, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    choice((
        just(Token::Identifier("int".to_string())).to(Type::Int),
        just(Token::Identifier("i8".to_string())).to(Type::Int),
        just(Token::Identifier("i16".to_string())).to(Type::Int),
        just(Token::Identifier("i32".to_string())).to(Type::Int),
        just(Token::Identifier("i64".to_string())).to(Type::Int),
        just(Token::Identifier("u8".to_string())).to(Type::Int),
        just(Token::Identifier("u16".to_string())).to(Type::Int),
        just(Token::Identifier("u32".to_string())).to(Type::Int),
        just(Token::Identifier("u64".to_string())).to(Type::Int),
        just(Token::Identifier("float".to_string())).to(Type::Float),
        just(Token::Identifier("f32".to_string())).to(Type::Float),
        just(Token::Identifier("f64".to_string())).to(Type::Float),
        just(Token::Identifier("bool".to_string())).to(Type::Bool),
        just(Token::Identifier("string".to_string())).to(Type::String),
        just(Token::Identifier("str".to_string())).to(Type::String),
    ))
    .delimited_by(just(Token::LParen), just(Token::RParen))
}

fn expr<'a, I>() -> impl Parser<'a, I, Expr, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    recursive(|expr| {
        let atom_base = choice((int_lit(), float_lit(), bool_lit(), string_lit(), unit_lit()));

        let range_array = signed_int()
            .then_ignore(just(Token::DotDot))
            .then(signed_int())
            .map(|(start, end)| Expr::new(ExprKind::RangeArray { start, end }))
            .delimited_by(just(Token::LBracket), just(Token::RBracket));
        let range_array = spanned_expr(range_array);

        let array = expr
            .clone()
            .separated_by(just(Token::Comma))
            .collect()
            .map(|e| Expr::new(ExprKind::Array(e)))
            .delimited_by(just(Token::LBracket), just(Token::RBracket));
        let array = spanned_expr(array);

        let paren = expr
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .map(|e| Expr::new(ExprKind::Paren(Box::new(e))));
        let paren = spanned_expr(paren);

        let call_args = expr
            .clone()
            .separated_by(just(Token::Comma))
            .collect()
            .delimited_by(just(Token::LParen), just(Token::RParen));

        let postfix = ident()
            .map(|id| Expr::new(ExprKind::Ident(id)))
            .then(
                choice((
                    just(Token::Dot)
                        .ignore_then(path_segment())
                        .or(just(Token::DoubleColon).ignore_then(path_segment()))
                        .map(Postfix::Field),
                    call_args.map(Postfix::Call),
                    just(Token::LBracket)
                        .ignore_then(expr.clone())
                        .then_ignore(just(Token::RBracket))
                        .map(Postfix::Index),
                ))
                .repeated()
                .collect::<Vec<_>>(),
            )
            .map(|(base, suffixes)| {
                suffixes.into_iter().fold(base, |acc, suffix| match suffix {
                    Postfix::Field(field) => Expr::new(ExprKind::Field {
                        object: Box::new(acc),
                        field,
                    }),
                    Postfix::Call(args) => Expr::new(ExprKind::Call {
                        func: Box::new(acc),
                        args,
                    }),
                    Postfix::Index(idx) => Expr::new(ExprKind::Index {
                        object: Box::new(acc),
                        index: Box::new(idx),
                    }),
                })
            });
        let postfix = spanned_expr(postfix);

        let atom = choice((postfix, range_array, array, paren, atom_base));

        #[derive(Clone)]
        enum Postfix {
            Field(String),
            Call(Vec<Expr>),
            Index(Expr),
        }

        spanned_expr(atom.pratt((
            prefix(8, explicit_cast_target(), |target, rhs, _| {
                Expr::new(ExprKind::Cast {
                    expr: Box::new(rhs),
                    target,
                })
            }),
            prefix(8, just(Token::Minus), |_, rhs, _| {
                Expr::new(ExprKind::UnaryOp {
                    op: UnOp::Neg,
                    expr: Box::new(rhs),
                })
            }),
            prefix(8, just(Token::Not), |_, rhs, _| {
                Expr::new(ExprKind::UnaryOp {
                    op: UnOp::Not,
                    expr: Box::new(rhs),
                })
            }),
            infix(right(7), just(Token::Caret), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Pow,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(6), just(Token::Star), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Mul,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(6), just(Token::Slash), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Div,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(6), just(Token::Percent), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Mod,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(5), just(Token::Plus), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Add,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(5), just(Token::Minus), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Sub,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(4), just(Token::Equals), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Eq,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(4), just(Token::NotEquals), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Neq,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(4), just(Token::Less), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Lt,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(4), just(Token::Greater), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Gt,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(4), just(Token::LessEq), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Le,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(4), just(Token::GreaterEq), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Ge,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(3), just(Token::And), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::And,
                    rhs: Box::new(rhs),
                })
            }),
            infix(left(2), just(Token::Or), |lhs, _, rhs, _| {
                Expr::new(ExprKind::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Or,
                    rhs: Box::new(rhs),
                })
            }),
        )))
    })
}

fn statement<'a, I>() -> impl Parser<'a, I, Statement, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    recursive(|stmt| {
        let read = just(Token::Read)
            .ignore_then(ident())
            .then_ignore(just(Token::Arrow))
            .then(ident())
            .map(|(sensor, variable)| Statement::Read {
                sensor,
                variable,
                span: None,
            });
        let read = spanned_stmt(read);

        let write_output_first = ident()
            .then_ignore(just(Token::Write))
            .then_ignore(just(Token::LeftArrow))
            .then(expr())
            .map(|(output, value)| Statement::Write {
                output,
                value,
                span: None,
            });
        let write_keyword_first = just(Token::Write)
            .ignore_then(ident())
            .then_ignore(just(Token::LeftArrow))
            .then(expr())
            .map(|(output, value)| Statement::Write {
                output,
                value,
                span: None,
            });
        let write = choice((write_output_first, write_keyword_first));
        let write = spanned_stmt(write);

        let sleep = just(Token::Sleep)
            .ignore_then(select! { Token::UnitLiteral(d) => d })
            .map(|raw: String| {
                let (value, unit) = parse_unit_str(&raw).unwrap_or((Number::Int(0), raw));
                Statement::Sleep {
                    value,
                    unit,
                    span: None,
                }
            });
        let sleep = spanned_stmt(sleep);

        let assignment =
            ident()
                .then_ignore(just(Token::Assign))
                .then(expr())
                .map(|(variable, value)| Statement::Assignment {
                    variable,
                    value,
                    span: None,
                });
        let assignment = spanned_stmt(assignment);

        let block = stmt
            .clone()
            .repeated()
            .collect()
            .delimited_by(just(Token::LBrace), just(Token::RBrace));

        let while_stmt = just(Token::While)
            .ignore_then(expr().delimited_by(just(Token::LParen), just(Token::RParen)))
            .then(block.clone())
            .map(|(condition, body)| Statement::While {
                condition,
                body,
                span: None,
            });
        let while_stmt = spanned_stmt(while_stmt);

        let for_stmt = just(Token::For)
            .ignore_then(ident())
            .then_ignore(just(Token::In))
            .then(expr())
            .then(block.clone())
            .map(|((variable, iterable), body)| Statement::For {
                variable,
                iterable,
                body,
                span: None,
            });
        let for_stmt = spanned_stmt(for_stmt);

        let else_clause = just(Token::Else)
            .ignore_then(choice((block.clone(), stmt.clone().map(|s| vec![s]))))
            .or_not();

        let if_stmt = just(Token::If)
            .ignore_then(expr().delimited_by(just(Token::LParen), just(Token::RParen)))
            .then(block.clone())
            .then(else_clause)
            .map(|((condition, then_body), else_body)| Statement::If {
                condition,
                then_body,
                else_body,
                span: None,
            });
        let if_stmt = spanned_stmt(if_stmt);

        let return_stmt = just(Token::Return)
            .ignore_then(expr().or_not())
            .map(|value| Statement::Return { value, span: None });
        let return_stmt = spanned_stmt(return_stmt);

        let expr_stmt = expr().map(Statement::Expr);
        let expr_stmt = spanned_stmt(expr_stmt);

        choice((
            read,
            write,
            if_stmt,
            while_stmt,
            for_stmt,
            sleep,
            return_stmt,
            assignment,
            expr_stmt,
        ))
    })
}

fn sensor_def<'a, I>() -> impl Parser<'a, I, TopLevel, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    spanned_top_level(
        just(Token::Sensor)
            .ignore_then(ident())
            .then_ignore(just(Token::On))
            .then(ident())
            .then(
                just(Token::Colon)
                    .ignore_then(ident())
                    .then_ignore(just(Token::Using))
                    .then(namespaced_ident())
                    .or_not(),
            )
            .map(|((name, pin), typed_tail)| {
                let (category, converter) = match typed_tail {
                    Some((category, converter)) => (Some(category), Some(converter)),
                    None => (None, None),
                };
                TopLevel::SensorDef(SensorDef {
                    name,
                    pin,
                    category,
                    converter,
                    span: None,
                })
            }),
    )
}

fn output_def<'a, I>() -> impl Parser<'a, I, TopLevel, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    spanned_top_level(
        just(Token::Output)
            .ignore_then(ident())
            .then_ignore(just(Token::On))
            .then(ident())
            .map(|(name, pin)| {
                TopLevel::OutputDef(OutputDef {
                    name,
                    pin,
                    span: None,
                })
            }),
    )
}

fn every_block<'a, I>() -> impl Parser<'a, I, TopLevel, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    let block = statement()
        .repeated()
        .collect()
        .delimited_by(just(Token::LBrace), just(Token::RBrace));

    spanned_top_level(
        just(Token::Every)
            .ignore_then(select! { Token::UnitLiteral(i) => i })
            .then(block)
            .map(|(raw, body)| {
                let (interval_value, interval_unit) =
                    parse_unit_str(&raw).unwrap_or((Number::Int(0), raw));
                TopLevel::Every(EveryBlock {
                    interval_value,
                    interval_unit,
                    body,
                    span: None,
                })
            }),
    )
}

fn task_block<'a, I>() -> impl Parser<'a, I, TopLevel, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    let block = statement()
        .repeated()
        .collect()
        .delimited_by(just(Token::LBrace), just(Token::RBrace));

    spanned_top_level(
        just(Token::Task)
            .ignore_then(block)
            .map(|body| TopLevel::Task(TaskBlock { body, span: None })),
    )
}

fn conversion_expr<'a, I>() -> impl Parser<'a, I, ConversionExpr, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    recursive(|conv_expr| {
        let val_atom = just(Token::Val).to(ConversionExpr::Val);

        let num_atom = choice((
            select! { Token::Int(n) => ConversionExpr::Lit(n as f64) },
            select! { Token::Float(f) => ConversionExpr::Lit(f) },
        ));

        let paren = conv_expr
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .map(|e| ConversionExpr::Paren(Box::new(e)));

        let atom = choice((val_atom, num_atom, paren));

        atom.pratt((
            prefix(5, just(Token::Minus), |_, rhs, _| {
                ConversionExpr::UnaryNeg(Box::new(rhs))
            }),
            infix(left(4), just(Token::Star), |lhs, _, rhs, _| {
                ConversionExpr::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Mul,
                    rhs: Box::new(rhs),
                }
            }),
            infix(left(4), just(Token::Slash), |lhs, _, rhs, _| {
                ConversionExpr::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Div,
                    rhs: Box::new(rhs),
                }
            }),
            infix(left(3), just(Token::Plus), |lhs, _, rhs, _| {
                ConversionExpr::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Add,
                    rhs: Box::new(rhs),
                }
            }),
            infix(left(3), just(Token::Minus), |lhs, _, rhs, _| {
                ConversionExpr::BinaryOp {
                    lhs: Box::new(lhs),
                    op: BinOp::Sub,
                    rhs: Box::new(rhs),
                }
            }),
        ))
    })
}

fn unit_def<'a, I>() -> impl Parser<'a, I, TopLevel, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    let conversion_pair = ident()
        .then_ignore(just(Token::Colon))
        .then(conversion_expr())
        .map(|(key, expr)| (key, expr));

    let conversions = conversion_pair
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LBrace), just(Token::RBrace));

    spanned_top_level(
        just(Token::UnitKw)
            .ignore_then(ident())
            .then_ignore(just(Token::Colon))
            .then(ident())
            .then(conversions)
            .map(|((name, category), conversions)| {
                TopLevel::UnitDef(UnitDef {
                    name,
                    category,
                    conversions,
                    span: None,
                })
            }),
    )
}

fn type_name<'a, I>() -> impl Parser<'a, I, String, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    choice((just(Token::VoidKw).to("void".to_string()), ident()))
}

fn extern_def<'a, I>() -> impl Parser<'a, I, TopLevel, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    let param = ident()
        .then_ignore(just(Token::Colon))
        .then(type_name())
        .map(|(name, ty)| (name, TypeAnnotation::new(ty)));

    let params = param
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen));

    spanned_top_level(
        just(Token::Extern)
            .ignore_then(namespaced_ident())
            .then(params)
            .then_ignore(just(Token::Arrow))
            .then(type_name())
            .map(|((name, params), ret)| {
                TopLevel::Extern(ExternDef {
                    name,
                    params,
                    ret: TypeAnnotation::new(ret),
                    span: None,
                })
            }),
    )
}

fn func_def<'a, I>() -> impl Parser<'a, I, TopLevel, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    let param = ident()
        .then_ignore(just(Token::Colon))
        .then(type_name())
        .map(|(name, ty)| (name, TypeAnnotation::new(ty)));

    let params = param
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen));

    let body = statement()
        .repeated()
        .collect()
        .delimited_by(just(Token::LBrace), just(Token::RBrace));

    spanned_top_level(
        just(Token::Fn)
            .ignore_then(ident())
            .then(params)
            .then_ignore(just(Token::Arrow))
            .then(type_name())
            .then(body)
            .map(|(((name, params), ret), body)| {
                TopLevel::FuncDef(FuncDef {
                    name,
                    params,
                    ret: TypeAnnotation::new(ret),
                    body,
                    span: None,
                })
            }),
    )
}

fn top_level<'a, I>() -> impl Parser<'a, I, TopLevel, ParseExtra<'a>> + Clone
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    choice((
        sensor_def(),
        every_block(),
        task_block(),
        output_def(),
        unit_def(),
        extern_def(),
        func_def(),
    ))
}

pub fn program_parser<'a, I>() -> impl Parser<'a, I, Program, ParseExtra<'a>>
where
    I: ValueInput<'a, Token = Token, Span = SimpleSpan<usize>>,
{
    top_level()
        .repeated()
        .collect()
        .map(|statements| Program { statements })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    #[derive(Debug)]
    struct ParseError;

    fn parse(input: &str) -> Result<Program, ParseError> {
        use chumsky::Parser;
        let lexer = Lexer::new();
        let tokens = lexer.tokenize_spanned(input).unwrap();
        let parser = program_parser();
        match parser
            .parse(token_stream(&tokens, input.len()))
            .into_result()
        {
            Ok(program) => Ok(program),
            Err(errs) => {
                eprintln!("Parse errors: {:?}", errs);
                Err(ParseError)
            }
        }
    }

    #[test]
    fn test_sensor_def() {
        let result = parse("sensor temp on A0").unwrap();
        assert_eq!(result.statements.len(), 1);
        match &result.statements[0] {
            TopLevel::SensorDef(s) => {
                assert_eq!(s.name, "temp");
                assert_eq!(s.pin, "A0");
                assert_eq!(s.category, None);
                assert_eq!(s.converter, None);
            }
            _ => panic!("Expected sensor def"),
        }
    }

    #[test]
    fn test_typed_sensor_def() {
        let result = parse("sensor temp on A0 : temperature using Sensor::convert").unwrap();
        assert_eq!(result.statements.len(), 1);
        match &result.statements[0] {
            TopLevel::SensorDef(s) => {
                assert_eq!(s.name, "temp");
                assert_eq!(s.pin, "A0");
                assert_eq!(s.category.as_deref(), Some("temperature"));
                assert_eq!(
                    s.converter.as_ref(),
                    Some(&vec!["Sensor".to_string(), "convert".to_string()])
                );
            }
            _ => panic!("Expected sensor def"),
        }
    }

    #[test]
    fn test_typed_sensor_def_missing_converter_is_rejected() {
        assert!(parse("sensor temp on A0 : temperature").is_err());
    }

    #[test]
    fn test_typed_sensor_def_missing_using_target_is_rejected() {
        assert!(parse("sensor temp on A0 : temperature using").is_err());
    }

    #[test]
    fn test_output_def() {
        let result = parse("output buzz on D0").unwrap();
        assert_eq!(result.statements.len(), 1);
        match &result.statements[0] {
            TopLevel::OutputDef(o) => {
                assert_eq!(o.name, "buzz");
                assert_eq!(o.pin, "D0");
            }
            _ => panic!("Expected output def"),
        }
    }

    #[test]
    fn test_every_block() {
        let result = parse("every 1s { }").unwrap();
        assert_eq!(result.statements.len(), 1);
        match &result.statements[0] {
            TopLevel::Every(e) => {
                assert_eq!(e.interval_value, crate::ast::Number::Int(1));
                assert_eq!(e.interval_unit, "s");
            }
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_task_block() {
        let result = parse("task { read temp -> t }").unwrap();
        assert_eq!(result.statements.len(), 1);
        match &result.statements[0] {
            TopLevel::Task(task) => {
                assert_eq!(task.body.len(), 1);
                match &task.body[0] {
                    Statement::Read {
                        sensor, variable, ..
                    } => {
                        assert_eq!(sensor, "temp");
                        assert_eq!(variable, "t");
                    }
                    _ => panic!("Expected read stmt"),
                }
            }
            _ => panic!("Expected task block"),
        }
    }

    #[test]
    fn test_task_block_malformed_is_rejected() {
        assert!(parse("task { read temp -> t ").is_err());
    }

    #[test]
    fn test_read_stmt() {
        let result = parse("every 1s { read temp -> t }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => {
                assert_eq!(e.body.len(), 1);
                match &e.body[0] {
                    Statement::Read {
                        sensor, variable, ..
                    } => {
                        assert_eq!(sensor, "temp");
                        assert_eq!(variable, "t");
                    }
                    _ => panic!("Expected read stmt"),
                }
            }
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_write_stmt_output_first() {
        let result = parse("every 1s { buzz write <- 255 }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => {
                assert_eq!(e.body.len(), 1);
                match &e.body[0] {
                    Statement::Write { output, value, .. } => {
                        assert_eq!(output, "buzz");
                        assert!(matches!(value.kind, ExprKind::IntLit(255)));
                    }
                    _ => panic!("Expected write stmt"),
                }
            }
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_write_stmt_keyword_first() {
        let result = parse("every 1s { write buzz <- 255 }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => {
                assert_eq!(e.body.len(), 1);
                match &e.body[0] {
                    Statement::Write { output, value, .. } => {
                        assert_eq!(output, "buzz");
                        assert!(matches!(value.kind, ExprKind::IntLit(255)));
                    }
                    _ => panic!("Expected write stmt"),
                }
            }
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_if_stmt() {
        let result = parse("every 1s { if (x > 5) { tone(buzz.pin) } }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::If {
                    condition,
                    then_body,
                    else_body,
                    ..
                } => {
                    assert!(matches!(condition.kind, ExprKind::BinaryOp { .. }));
                    assert_eq!(then_body.len(), 1);
                    assert!(else_body.is_none());
                }
                _ => panic!("Expected if stmt"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_if_else_stmt() {
        let result =
            parse("every 1s { if (x > 5) { tone(buzz.pin) } else { noTone(buzz.pin) } }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::If { else_body, .. } => {
                    assert!(else_body.is_some());
                }
                _ => panic!("Expected if stmt"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_while_stmt() {
        let result = parse("every 1s { while (x < 10) { piska() } }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::While { condition, .. } => {
                    assert!(matches!(condition.kind, ExprKind::BinaryOp { .. }));
                }
                _ => panic!("Expected while stmt"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_for_stmt() {
        let result = parse("every 1s { for i in [1, 2, 3] { print(i) } }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::For { variable, .. } => {
                    assert_eq!(variable, "i");
                }
                _ => panic!("Expected for stmt"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_for_stmt_with_range_array() {
        let result = parse("every 1s { for i in [0..5] { print(i) } }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::For {
                    variable, iterable, ..
                } => {
                    assert_eq!(variable, "i");
                    assert!(matches!(
                        iterable.kind,
                        ExprKind::RangeArray { start: 0, end: 5 }
                    ));
                }
                _ => panic!("Expected for stmt"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_range_array_with_negative_start() {
        let result = parse("every 1s { xs = [-2..3] }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::Assignment { value, .. } => {
                    assert!(matches!(
                        value.kind,
                        ExprKind::RangeArray { start: -2, end: 3 }
                    ));
                }
                _ => panic!("Expected assignment"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_sleep_stmt() {
        let result = parse("every 1s { sleep 5s }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::Sleep { value, unit, .. } => {
                    assert_eq!(*value, crate::ast::Number::Int(5));
                    assert_eq!(unit, "s");
                }
                _ => panic!("Expected sleep stmt"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_expr_stmt_rejects_trailing_semicolon() {
        assert!(parse("every 1s { am2302::read(); }").is_err());
    }

    #[test]
    fn test_assignment() {
        let result = parse("every 1s { x = 42 }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::Assignment {
                    variable, value, ..
                } => {
                    assert_eq!(variable, "x");
                    assert!(matches!(value.kind, ExprKind::IntLit(42)));
                }
                _ => panic!("Expected assignment"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_full_program() {
        let input = r#"
            sensor temp on A0
            sensor light on A1
            output buzz on D0

            every 1s {
                read temp -> t
                read light -> l

                if (t > 30) {
                    tone(buzz.pin, 1000)
                } else {
                    noTone(buzz.pin)
                }

                if (l < 100) {
                    sleep 5s
                }
            }
        "#;
        let result = parse(input).unwrap();
        assert_eq!(result.statements.len(), 4);
    }

    #[test]
    fn test_expr_precedence() {
        let result = parse("every 1s { x = 1 + 2 * 3 }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::Assignment { value, .. } => match &value.kind {
                    ExprKind::BinaryOp { op: BinOp::Add, .. } => (),
                    _ => panic!("Expected Add as root, got: {:?}", value),
                },
                _ => panic!("Expected assignment"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_paren_expr() {
        let result = parse("every 1s { x = (1 + 2) * 3 }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::Assignment { value, .. } => match &value.kind {
                    ExprKind::BinaryOp { op: BinOp::Mul, .. } => (),
                    _ => panic!("Expected Mul as root"),
                },
                _ => panic!("Expected assignment"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_explicit_cast_expr() {
        let result = parse("every 1s { x = (int)42ms }").unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::Assignment { value, .. } => match &value.kind {
                    ExprKind::Cast { expr, target } => {
                        assert_eq!(target, &crate::types::Type::Int);
                        assert!(matches!(expr.kind, ExprKind::UnitLit { .. }));
                    }
                    _ => panic!("Expected explicit cast expression"),
                },
                _ => panic!("Expected assignment"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_unit_def_simple() {
        let input = r#"
            unit kelvin : temperature {
                to_celsius: val - 273,
                from_celsius: val + 273
            }
        "#;
        let result = parse(input).unwrap();
        assert_eq!(result.statements.len(), 1);
        match &result.statements[0] {
            TopLevel::UnitDef(u) => {
                assert_eq!(u.name, "kelvin");
                assert_eq!(u.category, "temperature");
                assert_eq!(u.conversions.len(), 2);
                assert_eq!(u.conversions[0].0, "to_celsius");
                assert_eq!(u.conversions[1].0, "from_celsius");
            }
            _ => panic!("Expected unit def"),
        }
    }

    #[test]
    fn test_unit_def_complex_formula() {
        let input = r#"
            unit fahrenheit : temperature {
                to_celsius: (val - 32) * 5 / 9,
                from_celsius: val * 9 / 5 + 32
            }
        "#;
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::UnitDef(u) => {
                assert_eq!(u.name, "fahrenheit");
                assert_eq!(u.category, "temperature");
                assert_eq!(u.conversions.len(), 2);
            }
            _ => panic!("Expected unit def"),
        }
    }

    #[test]
    fn test_unit_def_with_program() {
        let input = r#"
            unit inches : distance {
                to_meters: val * 0.0254,
                from_meters: val / 0.0254
            }

            sensor temp on A0
            every 1s {
                read temp -> t
            }
        "#;
        let result = parse(input).unwrap();
        assert_eq!(result.statements.len(), 3);
        assert!(matches!(&result.statements[0], TopLevel::UnitDef(_)));
        assert!(matches!(&result.statements[1], TopLevel::SensorDef(_)));
        assert!(matches!(&result.statements[2], TopLevel::Every(_)));
    }

    #[test]
    fn test_unit_def_trailing_comma() {
        let input = r#"
            unit kelvin : temperature {
                to_celsius: val - 273,
                from_celsius: val + 273,
            }
        "#;
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::UnitDef(u) => {
                assert_eq!(u.conversions.len(), 2);
            }
            _ => panic!("Expected unit def"),
        }
    }

    #[test]
    fn test_extern_def() {
        let input = "extern sensor_read(id: u8) -> i16";
        let result = parse(input).unwrap();
        assert_eq!(result.statements.len(), 1);
        match &result.statements[0] {
            TopLevel::Extern(e) => {
                assert_eq!(e.name, vec!["sensor_read".to_string()]);
                assert_eq!(e.params.len(), 1);
                assert_eq!(e.params[0].0, "id");
                assert_eq!(e.params[0].1, TypeAnnotation::new("u8"));
                assert_eq!(e.ret, TypeAnnotation::new("i16"));
            }
            _ => panic!("Expected extern def"),
        }
    }

    #[test]
    fn test_extern_rejects_trailing_semicolon() {
        let input = "extern sensor_read(id: u8) -> i16;";
        assert!(parse(input).is_err());
    }

    #[test]
    fn test_extern_void_return() {
        let input = "extern delay_ms(ms: u32) -> void";
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::Extern(e) => {
                assert_eq!(e.name, vec!["delay_ms".to_string()]);
                assert_eq!(e.params.len(), 1);
                assert_eq!(e.params[0].0, "ms");
                assert_eq!(e.params[0].1, TypeAnnotation::new("u32"));
                assert_eq!(e.ret, TypeAnnotation::new("void"));
            }
            _ => panic!("Expected extern def"),
        }
    }

    #[test]
    fn test_extern_multiple_params() {
        let input = "extern spi_transfer(addr: u8, data: u16) -> i32";
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::Extern(e) => {
                assert_eq!(e.name, vec!["spi_transfer".to_string()]);
                assert_eq!(e.params.len(), 2);
                assert_eq!(e.params[0], ("addr".to_string(), TypeAnnotation::new("u8")));
                assert_eq!(
                    e.params[1],
                    ("data".to_string(), TypeAnnotation::new("u16"))
                );
                assert_eq!(e.ret, TypeAnnotation::new("i32"));
            }
            _ => panic!("Expected extern def"),
        }
    }

    #[test]
    fn test_extern_no_params() {
        let input = "extern get_tick() -> i32";
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::Extern(e) => {
                assert_eq!(e.name, vec!["get_tick".to_string()]);
                assert_eq!(e.params.len(), 0);
                assert_eq!(e.ret, TypeAnnotation::new("i32"));
            }
            _ => panic!("Expected extern def"),
        }
    }

    #[test]
    fn test_extern_namespaced_def() {
        let input = "extern Serial::println(msg: string) -> void";
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::Extern(e) => {
                assert_eq!(e.name, vec!["Serial".to_string(), "println".to_string()]);
                assert_eq!(
                    e.params[0],
                    ("msg".to_string(), TypeAnnotation::new("string"))
                );
                assert_eq!(e.ret, TypeAnnotation::new("void"));
            }
            _ => panic!("Expected extern def"),
        }
    }

    #[test]
    fn test_extern_namespaced_def_with_read_method() {
        let input = "extern am2302::read() -> void";
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::Extern(e) => {
                assert_eq!(e.name, vec!["am2302".to_string(), "read".to_string()]);
                assert_eq!(e.params.len(), 0);
                assert_eq!(e.ret, TypeAnnotation::new("void"));
            }
            _ => panic!("Expected extern def"),
        }
    }

    #[test]
    fn test_extern_namespaced_def_with_keyword_method() {
        let input = "extern device::sleep() -> void";
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::Extern(e) => {
                assert_eq!(e.name, vec!["device".to_string(), "sleep".to_string()]);
                assert_eq!(e.params.len(), 0);
                assert_eq!(e.ret, TypeAnnotation::new("void"));
            }
            _ => panic!("Expected extern def"),
        }
    }

    #[test]
    fn test_namespaced_call_with_read_method() {
        let input = "every 5s { am2302::read() }";
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::Expr(expr) => match &expr.kind {
                    ExprKind::Call { func, args } => {
                        assert!(args.is_empty());
                        match &func.kind {
                            ExprKind::Field { object, field } => {
                                assert_eq!(field, "read");
                                assert!(
                                    matches!(&object.kind, ExprKind::Ident(name) if name == "am2302")
                                );
                            }
                            _ => panic!("Expected namespaced call target"),
                        }
                    }
                    _ => panic!("Expected call expression"),
                },
                _ => panic!("Expected expression statement"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_namespaced_call_with_keyword_method() {
        let input = "every 5s { device::sleep() }";
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::Every(e) => match &e.body[0] {
                Statement::Expr(expr) => match &expr.kind {
                    ExprKind::Call { func, args } => {
                        assert!(args.is_empty());
                        match &func.kind {
                            ExprKind::Field { object, field } => {
                                assert_eq!(field, "sleep");
                                assert!(
                                    matches!(&object.kind, ExprKind::Ident(name) if name == "device")
                                );
                            }
                            _ => panic!("Expected namespaced call target"),
                        }
                    }
                    _ => panic!("Expected call expression"),
                },
                _ => panic!("Expected expression statement"),
            },
            _ => panic!("Expected every block"),
        }
    }

    #[test]
    fn test_extern_namespaced_def_rejects_c_keyword_method() {
        let input = "extern device::return() -> void";
        assert!(parse(input).is_err());
    }

    #[test]
    fn test_namespaced_call_rejects_c_keyword_method() {
        let input = "every 5s { device::while() }";
        assert!(parse(input).is_err());
    }

    #[test]
    fn test_extern_with_program() {
        let input = r#"
            extern sensor_read(id: u8) -> i16
            extern delay_ms(ms: u32) -> void
            sensor temp on A0
            every 1s {
                read temp -> t
            }
        "#;
        let result = parse(input).unwrap();
        assert_eq!(result.statements.len(), 4);
        assert!(matches!(&result.statements[0], TopLevel::Extern(_)));
        assert!(matches!(&result.statements[1], TopLevel::Extern(_)));
        assert!(matches!(&result.statements[2], TopLevel::SensorDef(_)));
        assert!(matches!(&result.statements[3], TopLevel::Every(_)));
    }

    #[test]
    fn test_func_def_simple() {
        let input = r#"
            fn add(a: int, b: int) -> int {
                return a + b
            }
        "#;
        let result = parse(input).unwrap();
        assert_eq!(result.statements.len(), 1);
        match &result.statements[0] {
            TopLevel::FuncDef(f) => {
                assert_eq!(f.name, "add");
                assert_eq!(f.params.len(), 2);
                assert_eq!(f.params[0], ("a".to_string(), TypeAnnotation::new("int")));
                assert_eq!(f.params[1], ("b".to_string(), TypeAnnotation::new("int")));
                assert_eq!(f.ret, TypeAnnotation::new("int"));
                assert_eq!(f.body.len(), 1);
                assert!(matches!(
                    &f.body[0],
                    Statement::Return { value: Some(_), .. }
                ));
            }
            _ => panic!("Expected func def"),
        }
    }

    #[test]
    fn test_func_def_void() {
        let input = r#"
            fn do_nothing() -> void {
                return
            }
        "#;
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::FuncDef(f) => {
                assert_eq!(f.name, "do_nothing");
                assert_eq!(f.params.len(), 0);
                assert_eq!(f.ret, TypeAnnotation::new("void"));
                assert_eq!(f.body.len(), 1);
                assert!(matches!(&f.body[0], Statement::Return { value: None, .. }));
            }
            _ => panic!("Expected func def"),
        }
    }

    #[test]
    fn test_func_def_with_body() {
        let input = r#"
            fn average(a: int, b: int) -> int {
                result = (a + b) / 2
                return result
            }
        "#;
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::FuncDef(f) => {
                assert_eq!(f.name, "average");
                assert_eq!(f.body.len(), 2);
                assert!(matches!(&f.body[0], Statement::Assignment { .. }));
                assert!(matches!(
                    &f.body[1],
                    Statement::Return { value: Some(_), .. }
                ));
            }
            _ => panic!("Expected func def"),
        }
    }

    #[test]
    fn test_func_def_with_program() {
        let input = r#"
            fn double(x: int) -> int {
                return x * 2
            }

            sensor temp on A0
            every 1s {
                read temp -> t
                d = double(t)
            }
        "#;
        let result = parse(input).unwrap();
        assert_eq!(result.statements.len(), 3);
        assert!(matches!(&result.statements[0], TopLevel::FuncDef(_)));
        assert!(matches!(&result.statements[1], TopLevel::SensorDef(_)));
        assert!(matches!(&result.statements[2], TopLevel::Every(_)));
    }

    #[test]
    fn test_return_stmt_in_if() {
        let input = r#"
            fn abs(x: int) -> int {
                if (x < 0) {
                    return -x
                }
                return x
            }
        "#;
        let result = parse(input).unwrap();
        match &result.statements[0] {
            TopLevel::FuncDef(f) => {
                assert_eq!(f.body.len(), 2);
                match &f.body[0] {
                    Statement::If { then_body, .. } => {
                        assert!(matches!(
                            &then_body[0],
                            Statement::Return { value: Some(_), .. }
                        ));
                    }
                    _ => panic!("Expected if stmt"),
                }
                assert!(matches!(
                    &f.body[1],
                    Statement::Return { value: Some(_), .. }
                ));
            }
            _ => panic!("Expected func def"),
        }
    }
}
