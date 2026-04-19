use crate::ast::Number;
use crate::diagnostics::SourceSpan;
use crate::types::{Type, UnitCategory};

#[derive(Debug, Clone)]
pub struct IrProgram {
    pub definitions: Vec<IrDefinition>,
    pub tasks: Vec<IrTask>,
    pub setup_body: Vec<IrStmt>,
    pub functions: Vec<IrFunction>,
    pub scheduler: SchedulerConfig,
    pub energy_policy: EnergyPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulerModel {
    TickDriven,
}

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub model: SchedulerModel,
    pub tick_micros: u32,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            model: SchedulerModel::TickDriven,
            tick_micros: 1000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerState {
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeSource {
    Tick,
    Timer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SleepFallback {
    UseActiveDelay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrunPolicy {
    Skip,
}

#[derive(Debug, Clone)]
pub struct EnergyPolicy {
    pub default_sleep_state: PowerState,
    pub allowed_wake_sources: Vec<WakeSource>,
    pub overrun_policy: OverrunPolicy,
}

impl Default for EnergyPolicy {
    fn default() -> Self {
        Self {
            default_sleep_state: PowerState::Idle,
            allowed_wake_sources: vec![WakeSource::Tick],
            overrun_policy: OverrunPolicy::Skip,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskTrigger {
    Periodic { period_ticks: u32, phase_ticks: u32 },
}

#[derive(Debug, Clone)]
pub enum IrDefinition {
    Sensor(IrSensor),
    Output(IrOutput),
    Unit(IrUnitDef),
    Extern(IrExtern),
}
#[derive(Debug, Clone)]
pub struct IrSensor {
    pub name: String,
    pub pin: String,
    pub category: Option<UnitCategory>,
    pub converter: Option<Vec<String>>,
}
#[derive(Debug, Clone)]
pub struct IrOutput {
    pub name: String,
    pub pin: String,
}
#[derive(Debug, Clone)]
pub struct IrUnitDef {
    pub name: String,
    pub category: UnitCategory,
    pub to_base: i64,
    pub from_base: i64,
}
#[derive(Debug, Clone)]
pub struct IrExtern {
    pub name: Vec<String>,
    pub params: Vec<(String, IrType)>,
    pub return_type: IrType,
}

#[derive(Debug, Clone)]
pub struct IrTask {
    pub name: Option<String>,
    pub trigger: TaskTrigger,
    pub body: Vec<IrStmt>,
    pub source_span: Option<SourceSpan>,
}

#[derive(Debug, Clone)]
pub struct IrFunction {
    pub name: String,
    pub params: Vec<(String, IrType)>,
    pub return_type: IrType,
    pub body: Vec<IrStmt>,
    pub source_span: Option<SourceSpan>,
}

#[derive(Debug, Clone)]
pub enum IrStmt {
    Read {
        sensor: String,
        variable: String,
        value_type: IrType,
        converter: Option<Vec<String>>,
        source_span: Option<SourceSpan>,
    },
    Write {
        output: String,
        value: IrExpr,
        source_span: Option<SourceSpan>,
    },
    Sleep {
        duration_micros: Option<i64>,
        mode_hint: Option<PowerState>,
        wake_sources: Vec<WakeSource>,
        fallback: SleepFallback,
        source_span: Option<SourceSpan>,
    },
    If {
        condition: IrExpr,
        then_body: Vec<IrStmt>,
        else_body: Option<Vec<IrStmt>>,
        source_span: Option<SourceSpan>,
    },
    While {
        condition: IrExpr,
        body: Vec<IrStmt>,
        source_span: Option<SourceSpan>,
    },
    For {
        variable: String,
        iterable: IrExpr,
        body: Vec<IrStmt>,
        source_span: Option<SourceSpan>,
    },
    Assign {
        variable: String,
        value: IrExpr,
        source_span: Option<SourceSpan>,
    },
    Return {
        value: Option<IrExpr>,
        source_span: Option<SourceSpan>,
    },
    Expr(IrExpr),
    PeriodicBlock {
        body: Vec<IrStmt>,
        source_span: Option<SourceSpan>,
    },
}

#[derive(Debug, Clone)]
pub struct IrExpr {
    pub kind: IrExprKind,
    pub ty: IrType,
    pub unit: Option<UnitCategory>,
    pub source_span: Option<SourceSpan>,
}

#[derive(Debug, Clone)]
pub enum IrExprKind {
    IntLit(i64),
    FloatLit(f64),
    BoolLit(bool),
    StringLit(String),
    UnitLit {
        value: Number,
        unit: String,
        category: UnitCategory,
    },
    Ident(String),
    BinaryOp {
        lhs: Box<IrExpr>,
        op: IrBinOp,
        rhs: Box<IrExpr>,
    },
    UnaryOp {
        op: IrUnOp,
        expr: Box<IrExpr>,
    },
    Cast {
        expr: Box<IrExpr>,
        target: IrType,
    },
    Call {
        func: Vec<String>,
        args: Vec<IrExpr>,
    },
    Array(Vec<IrExpr>),
    Index {
        object: Box<IrExpr>,
        index: Box<IrExpr>,
    },
    Field {
        object: Box<IrExpr>,
        field: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrBinOp {
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
pub enum IrUnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IrType {
    Int,
    Float,
    Bool,
    Unit(UnitCategory),
    Pin,
    String,
    Array(Box<IrType>),
    Sensor,
    Output,
    Void,
    Unknown,
}

impl IrType {
    pub fn from_ast(ty: &Type) -> Self {
        match ty {
            Type::Int => IrType::Int,
            Type::Float => IrType::Float,
            Type::Bool => IrType::Bool,
            Type::Unit(cat) => IrType::Unit(cat.clone()),
            Type::Pin => IrType::Pin,
            Type::Sensor => IrType::Sensor,
            Type::Output => IrType::Output,
            Type::Void => IrType::Void,
            Type::Unknown => IrType::Int,
            Type::Array(elem) => IrType::Array(Box::new(Self::from_ast(elem))),
            Type::Function(_, ret) => Self::from_ast(ret),
            Type::String => IrType::String,
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, IrType::Int | IrType::Float)
    }
    #[cfg(test)]
    pub fn is_time(&self) -> bool {
        matches!(self, IrType::Unit(UnitCategory::Time))
    }
}

impl std::fmt::Display for IrType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IrType::Int => write!(f, "int"),
            IrType::Float => write!(f, "float"),
            IrType::Bool => write!(f, "bool"),
            IrType::Unit(cat) => write!(f, "{}", cat),
            IrType::Pin => write!(f, "pin"),
            IrType::Sensor => write!(f, "sensor"),
            IrType::Array(elem) => write!(f, "array[{}]", elem),
            IrType::String => write!(f, "string"),
            IrType::Output => write!(f, "output"),
            IrType::Void => write!(f, "void"),
            IrType::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum IrError {
    SleepInForbiddenContext {
        source_span: Option<SourceSpan>,
    },
    SleepOutsidePeriodicBlock {
        source_span: Option<SourceSpan>,
    },
    TypeMismatch {
        expected: IrType,
        found: IrType,
        source_span: Option<SourceSpan>,
    },
    InvalidTaskPeriod {
        period_micros: i64,
        source_span: Option<SourceSpan>,
    },
    DivisionByZero {
        source_span: Option<SourceSpan>,
    },
    UnknownVariable {
        name: String,
        source_span: Option<SourceSpan>,
    },
    UnknownSensor {
        name: String,
        source_span: Option<SourceSpan>,
    },
    NoTasksDefined,
}

impl std::fmt::Display for IrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IrError::SleepInForbiddenContext { source_span } => {
                write!(f, "Sleep is not allowed in loops (at {:?})", source_span)
            }
            IrError::SleepOutsidePeriodicBlock { source_span } => {
                write!(
                    f,
                    "Sleep is only allowed inside periodic or task blocks (at {:?})",
                    source_span
                )
            }
            IrError::TypeMismatch {
                expected,
                found,
                source_span,
            } => {
                write!(
                    f,
                    "Type mismatch: expected {}, found {} (at {:?})",
                    expected, found, source_span
                )
            }
            IrError::InvalidTaskPeriod {
                period_micros,
                source_span,
            } => {
                write!(
                    f,
                    "Invalid task period: {} microseconds (at {:?})",
                    period_micros, source_span
                )
            }
            IrError::DivisionByZero { source_span } => {
                write!(f, "Division by zero (at {:?})", source_span)
            }
            IrError::UnknownVariable { name, source_span } => {
                write!(f, "Unknown variable: {} (at {:?})", name, source_span)
            }
            IrError::UnknownSensor { name, source_span } => {
                write!(f, "Unknown sensor: {} (at {:?})", name, source_span)
            }
            IrError::NoTasksDefined => {
                write!(f, "No executable blocks defined in the program")
            }
        }
    }
}

impl IrError {
    pub fn source_span(&self) -> Option<SourceSpan> {
        match self {
            IrError::SleepInForbiddenContext { source_span }
            | IrError::SleepOutsidePeriodicBlock { source_span }
            | IrError::InvalidTaskPeriod { source_span, .. }
            | IrError::DivisionByZero { source_span }
            | IrError::UnknownVariable { source_span, .. }
            | IrError::UnknownSensor { source_span, .. } => *source_span,
            IrError::TypeMismatch { source_span, .. } => *source_span,
            IrError::NoTasksDefined => None,
        }
    }

    pub fn with_fallback_span(self, fallback_span: Option<SourceSpan>) -> Self {
        match self {
            IrError::SleepInForbiddenContext { source_span } => IrError::SleepInForbiddenContext {
                source_span: source_span.or(fallback_span),
            },
            IrError::SleepOutsidePeriodicBlock { source_span } => {
                IrError::SleepOutsidePeriodicBlock {
                    source_span: source_span.or(fallback_span),
                }
            }
            IrError::TypeMismatch {
                expected,
                found,
                source_span,
            } => IrError::TypeMismatch {
                expected,
                found,
                source_span: source_span.or(fallback_span),
            },
            IrError::InvalidTaskPeriod {
                period_micros,
                source_span,
            } => IrError::InvalidTaskPeriod {
                period_micros,
                source_span: source_span.or(fallback_span),
            },
            IrError::DivisionByZero { source_span } => IrError::DivisionByZero {
                source_span: source_span.or(fallback_span),
            },
            IrError::UnknownVariable { name, source_span } => IrError::UnknownVariable {
                name,
                source_span: source_span.or(fallback_span),
            },
            IrError::UnknownSensor { name, source_span } => IrError::UnknownSensor {
                name,
                source_span: source_span.or(fallback_span),
            },
            IrError::NoTasksDefined => IrError::NoTasksDefined,
        }
    }
}

impl std::error::Error for IrError {}

pub type IrResult<T> = Result<T, IrError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrBlockKind {
    Loop,
    Periodic,
    Setup,
}

#[derive(Debug, Clone, Default)]
pub struct IrLoweringContext {
    pub sensors: std::collections::HashMap<String, IrSensorReadInfo>,
    pub outputs: std::collections::HashSet<String>,
    pub variables: std::collections::HashSet<String>,
    pub block_stack: Vec<IrBlockKind>,
    pub unit_registry: crate::types::UnitRegistry,
}

#[derive(Debug, Clone)]
pub struct IrSensorReadInfo {
    pub value_type: IrType,
    pub converter: Option<Vec<String>>,
}

impl IrLoweringContext {
    pub fn new() -> Self {
        Self {
            sensors: std::collections::HashMap::new(),
            outputs: std::collections::HashSet::new(),
            variables: std::collections::HashSet::new(),
            block_stack: Vec::new(),
            unit_registry: crate::types::UnitRegistry::new(),
        }
    }
    pub fn add_sensor(&mut self, name: impl Into<String>, read_info: IrSensorReadInfo) {
        self.sensors.insert(name.into(), read_info);
    }
    pub fn add_output(&mut self, name: impl Into<String>) {
        self.outputs.insert(name.into());
    }
    pub fn add_variable(&mut self, name: impl Into<String>) {
        self.variables.insert(name.into());
    }
    pub fn has_sensor(&self, name: &str) -> bool {
        self.sensors.contains_key(name)
    }
    pub fn sensor_read_info(&self, name: &str) -> Option<&IrSensorReadInfo> {
        self.sensors.get(name)
    }
    pub fn has_output(&self, name: &str) -> bool {
        self.outputs.contains(name)
    }
    pub fn has_variable(&self, name: &str) -> bool {
        self.variables.contains(name)
    }
    pub fn enter_block(&mut self, kind: IrBlockKind) {
        self.block_stack.push(kind);
    }

    pub fn exit_block(&mut self) {
        let _ = self.block_stack.pop();
    }

    pub fn is_in_loop(&self) -> bool {
        self.block_stack
            .iter()
            .any(|k| matches!(k, IrBlockKind::Loop))
    }

    pub fn is_in_periodic_block(&self) -> bool {
        self.block_stack
            .iter()
            .any(|k| matches!(k, IrBlockKind::Periodic))
    }

    pub fn is_in_setup_block(&self) -> bool {
        self.block_stack
            .iter()
            .any(|k| matches!(k, IrBlockKind::Setup))
    }

    pub fn can_sleep_here(&self) -> bool {
        (self.is_in_periodic_block() || self.is_in_setup_block()) && !self.is_in_loop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ir_type_from_ast() {
        assert_eq!(IrType::from_ast(&Type::Int), IrType::Int);
        assert_eq!(IrType::from_ast(&Type::Float), IrType::Float);
        assert_eq!(IrType::from_ast(&Type::Bool), IrType::Bool);
        assert_eq!(
            IrType::from_ast(&Type::Unit(UnitCategory::Time)),
            IrType::Unit(UnitCategory::Time)
        );
        assert_eq!(IrType::from_ast(&Type::Void), IrType::Void);
    }

    #[test]
    fn test_ir_type_is_numeric() {
        assert!(IrType::Int.is_numeric());
        assert!(IrType::Float.is_numeric());
        assert!(!IrType::Bool.is_numeric());
        assert!(!IrType::Unit(UnitCategory::Time).is_numeric());
    }

    #[test]
    fn test_ir_type_is_time() {
        assert!(IrType::Unit(UnitCategory::Time).is_time());
        assert!(!IrType::Unit(UnitCategory::Temperature).is_time());
        assert!(!IrType::Unit(UnitCategory::Custom("pressure".to_string())).is_time());
        assert!(!IrType::Int.is_time());
    }

    #[test]
    fn test_lowering_context() {
        let mut ctx = IrLoweringContext::new();
        ctx.add_sensor(
            "temp_sensor".to_string(),
            IrSensorReadInfo {
                value_type: IrType::Int,
                converter: None,
            },
        );
        ctx.add_output("led".to_string());
        ctx.add_variable("x".to_string());

        assert!(ctx.has_sensor("temp_sensor"));
        assert!(ctx.has_output("led"));
        assert!(ctx.has_variable("x"));
        assert!(!ctx.has_sensor("unknown"));
    }

    #[test]
    fn test_loop_context() {
        let mut ctx = IrLoweringContext::new();
        assert!(!ctx.is_in_loop());
        assert!(!ctx.is_in_periodic_block());
        assert!(!ctx.is_in_setup_block());
        assert!(!ctx.can_sleep_here());

        ctx.enter_block(IrBlockKind::Periodic);
        assert!(!ctx.is_in_loop());
        assert!(ctx.is_in_periodic_block());
        assert!(ctx.can_sleep_here());

        ctx.enter_block(IrBlockKind::Loop);
        assert!(ctx.is_in_loop());
        assert!(ctx.is_in_periodic_block());
        assert!(!ctx.can_sleep_here());

        ctx.exit_block();
        assert!(!ctx.is_in_loop());
        assert!(ctx.is_in_periodic_block());
        assert!(ctx.can_sleep_here());

        ctx.exit_block();
        assert!(!ctx.is_in_periodic_block());
        assert!(!ctx.is_in_setup_block());
        assert!(!ctx.can_sleep_here());

        ctx.enter_block(IrBlockKind::Setup);
        assert!(ctx.is_in_setup_block());
        assert!(ctx.can_sleep_here());

        ctx.enter_block(IrBlockKind::Loop);
        assert!(ctx.is_in_loop());
        assert!(!ctx.can_sleep_here());

        ctx.exit_block();
        assert!(!ctx.is_in_loop());
        assert!(ctx.is_in_setup_block());
        assert!(ctx.can_sleep_here());

        ctx.exit_block();
        assert!(!ctx.is_in_setup_block());
        assert!(!ctx.can_sleep_here());
    }

    #[test]
    fn test_scheduler_defaults() {
        let cfg = SchedulerConfig::default();
        assert_eq!(cfg.model, SchedulerModel::TickDriven);
        assert_eq!(cfg.tick_micros, 1000);

        let policy = EnergyPolicy::default();
        assert_eq!(policy.default_sleep_state, PowerState::Idle);
        assert_eq!(policy.allowed_wake_sources, vec![WakeSource::Tick]);
        assert_eq!(policy.overrun_policy, OverrunPolicy::Skip);
    }
}
