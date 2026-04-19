use crate::diagnostics::SourceSpan;
use crate::types::{Type, UnitCategory};

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<TopLevel>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopLevel {
    SensorDef(SensorDef),
    OutputDef(OutputDef),
    UnitDef(UnitDef),
    Extern(ExternDef),
    FuncDef(FuncDef),
    Every(EveryBlock),
    Task(TaskBlock),
}

impl TopLevel {
    pub fn with_span(mut self, span: SourceSpan) -> Self {
        match &mut self {
            TopLevel::SensorDef(def) => def.span = Some(span),
            TopLevel::OutputDef(def) => def.span = Some(span),
            TopLevel::UnitDef(def) => def.span = Some(span),
            TopLevel::Extern(def) => def.span = Some(span),
            TopLevel::FuncDef(def) => def.span = Some(span),
            TopLevel::Every(def) => def.span = Some(span),
            TopLevel::Task(def) => def.span = Some(span),
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeAnnotation(pub String);

impl TypeAnnotation {
    pub fn new(s: impl Into<String>) -> Self {
        TypeAnnotation(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TypeAnnotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SensorDef {
    pub name: String,
    pub pin: String,
    pub category: Option<String>,
    pub converter: Option<Vec<String>>,
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputDef {
    pub name: String,
    pub pin: String,
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EveryBlock {
    pub interval_value: Number,
    pub interval_unit: String,
    pub body: Vec<Statement>,
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskBlock {
    pub body: Vec<Statement>,
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternDef {
    pub name: Vec<String>,
    pub params: Vec<(String, TypeAnnotation)>,
    pub ret: TypeAnnotation,
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<(String, TypeAnnotation)>,
    pub ret: TypeAnnotation,
    pub body: Vec<Statement>,
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnitDef {
    pub name: String,
    pub category: String,
    pub conversions: Vec<(String, ConversionExpr)>,
    pub span: Option<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConversionExpr {
    Val,
    Lit(f64),
    BinaryOp {
        lhs: Box<ConversionExpr>,
        op: BinOp,
        rhs: Box<ConversionExpr>,
    },
    Paren(Box<ConversionExpr>),
    UnaryNeg(Box<ConversionExpr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Read {
        sensor: String,
        variable: String,
        span: Option<SourceSpan>,
    },
    Write {
        output: String,
        value: Expr,
        span: Option<SourceSpan>,
    },
    If {
        condition: Expr,
        then_body: Vec<Statement>,
        else_body: Option<Vec<Statement>>,
        span: Option<SourceSpan>,
    },
    While {
        condition: Expr,
        body: Vec<Statement>,
        span: Option<SourceSpan>,
    },
    For {
        variable: String,
        iterable: Expr,
        body: Vec<Statement>,
        span: Option<SourceSpan>,
    },
    Sleep {
        value: Number,
        unit: String,
        span: Option<SourceSpan>,
    },
    Assignment {
        variable: String,
        value: Expr,
        span: Option<SourceSpan>,
    },
    Return {
        value: Option<Expr>,
        span: Option<SourceSpan>,
    },
    Expr(Expr),
}

impl Statement {
    pub fn with_span(mut self, span: SourceSpan) -> Self {
        match &mut self {
            Statement::Read { span: slot, .. }
            | Statement::Write { span: slot, .. }
            | Statement::If { span: slot, .. }
            | Statement::While { span: slot, .. }
            | Statement::For { span: slot, .. }
            | Statement::Sleep { span: slot, .. }
            | Statement::Assignment { span: slot, .. }
            | Statement::Return { span: slot, .. } => *slot = Some(span),
            Statement::Expr(expr) => expr.span = Some(span),
        }
        self
    }

    pub fn span(&self) -> Option<SourceSpan> {
        match self {
            Statement::Read { span, .. }
            | Statement::Write { span, .. }
            | Statement::If { span, .. }
            | Statement::While { span, .. }
            | Statement::For { span, .. }
            | Statement::Sleep { span, .. }
            | Statement::Assignment { span, .. }
            | Statement::Return { span, .. } => *span,
            Statement::Expr(expr) => expr.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Number {
    Int(i64),
    Float(f64),
}

impl std::fmt::Display for Number {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Number::Int(i) => write!(f, "{}", i),
            Number::Float(flt) => write!(f, "{}", flt),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Option<Type>,
    pub unit: Option<UnitCategory>,
    pub span: Option<SourceSpan>,
}

impl Expr {
    pub fn new(kind: ExprKind) -> Self {
        Self {
            kind,
            ty: None,
            unit: None,
            span: None,
        }
    }

    pub fn with_span(mut self, span: SourceSpan) -> Self {
        self.span = Some(span);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    IntLit(i64),
    FloatLit(f64),
    BoolLit(bool),
    StringLit(String),
    UnitLit {
        value: Number,
        unit: String,
    },
    Ident(String),
    BinaryOp {
        lhs: Box<Expr>,
        op: BinOp,
        rhs: Box<Expr>,
    },
    UnaryOp {
        op: UnOp,
        expr: Box<Expr>,
    },
    Cast {
        expr: Box<Expr>,
        target: Type,
    },
    RangeArray {
        start: i64,
        end: i64,
    },
    Array(Vec<Expr>),
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    Paren(Box<Expr>),
    Call {
        func: Box<Expr>,
        args: Vec<Expr>,
    },
    Field {
        object: Box<Expr>,
        field: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Neq,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}
