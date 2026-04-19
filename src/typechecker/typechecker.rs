use crate::ast::*;
use crate::diagnostics::SourceSpan;
use crate::types::{
    CustomUnitDef, Type, UnitCategory, UnitRegistry, builtin_time_to_micros, parse_category,
    parse_type_name,
};
use std::collections::{HashMap, HashSet};

const C11_KEYWORDS: &[&str] = &[
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "register",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "typedef",
    "union",
    "unsigned",
    "void",
    "volatile",
    "while",
    "_Alignas",
    "_Alignof",
    "_Atomic",
    "_Bool",
    "_Complex",
    "_Generic",
    "_Imaginary",
    "_Noreturn",
    "_Static_assert",
    "_Thread_local",
];

fn c_codegen_identifier_violation(name: &str) -> Option<&'static str> {
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Some("must match [A-Za-z_][A-Za-z0-9_]*");
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Some("must match [A-Za-z_][A-Za-z0-9_]*");
    }
    if C11_KEYWORDS.iter().any(|kw| kw == &name) {
        return Some("is a reserved C11 keyword");
    }
    if name.starts_with("__")
        || name.starts_with("__lpc_")
        || (name.starts_with('_')
            && name
                .chars()
                .nth(1)
                .is_some_and(|second| second == '_' || second.is_ascii_uppercase()))
    {
        return Some("uses an implementation-reserved C identifier pattern");
    }

    None
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeError {
    pub message: String,
    pub span: Option<SourceSpan>,
}

impl TypeError {
    fn at(msg: impl Into<String>, span: Option<SourceSpan>) -> Self {
        TypeError {
            message: msg.into(),
            span,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FunctionSig {
    pub params: Vec<Type>,
    pub ret: Type,
}

#[derive(Debug, Clone)]
struct CustomTimeConversion {
    target_unit: String,
    to_target_expr: ConversionExpr,
}

#[derive(Debug, Clone)]
struct SensorReadType {
    read_type: Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TimeConversionError {
    Overflow { unit: String },
    Cycle { unit: String },
    UnknownUnit { unit: String },
    InvalidConversion { unit: String },
    NonFinite { unit: String },
}

pub struct TypeChecker {
    symbols: HashMap<String, Type>,
    errors: Vec<TypeError>,
    sensor_read_types: HashMap<String, SensorReadType>,
    unit_registry: UnitRegistry,
    custom_time_conversions: HashMap<String, CustomTimeConversion>,
    category_base_targets: HashMap<UnitCategory, String>,
    unit_base_targets: HashMap<String, String>,
    externals: HashMap<String, FunctionSig>,
    functions: HashMap<String, FunctionSig>,
    current_return_type: Option<Type>,
    current_every_period_micros: Option<i64>,
    task_blocks_seen: usize,
    inside_loop: bool,
}

impl TypeChecker {
    fn validate_c_codegen_identifier(&mut self, name: &str, span: Option<SourceSpan>) -> bool {
        if name.is_empty() {
            self.errors.push(TypeError::at(
                "unsafe identifier for C codegen: '' (identifier cannot be empty)",
                span,
            ));
            return false;
        }

        if let Some(reason) = c_codegen_identifier_violation(name) {
            self.errors.push(TypeError::at(
                format!("unsafe identifier for C codegen: '{}' ({})", name, reason),
                span,
            ));
            return false;
        }

        true
    }

    fn validate_c_codegen_path(
        &mut self,
        segments: &[String],
        span: Option<SourceSpan>,
        context: &str,
    ) -> bool {
        if segments.is_empty() {
            self.errors.push(TypeError::at(
                format!(
                    "unsafe identifier for C codegen: '<empty {} path>'",
                    context
                ),
                span,
            ));
            return false;
        }

        let mut ok = true;
        for segment in segments {
            if !self.validate_c_codegen_identifier(segment, span) {
                ok = false;
            }
        }
        ok
    }

    fn validate_c_codegen_call_expr_path(
        &mut self,
        expr: &Expr,
        span: Option<SourceSpan>,
    ) -> Option<String> {
        let path = call_path(expr)?;
        let segments = path
            .split("::")
            .map(ToOwned::to_owned)
            .collect::<Vec<String>>();
        self.validate_c_codegen_path(&segments, span, "call");
        Some(path)
    }

    fn format_number(value: &Number) -> String {
        match value {
            Number::Int(i) => i.to_string(),
            Number::Float(f) => f.to_string(),
        }
    }

    fn is_builtin_time_unit(unit: &str) -> bool {
        matches!(unit, "us" | "μs" | "ms" | "s" | "min" | "h")
    }

    fn time_conversion_error_message(
        literal_value: &Number,
        literal_unit: &str,
        err: &TimeConversionError,
    ) -> String {
        let literal = format!("{}{}", Self::format_number(literal_value), literal_unit);
        match err {
            TimeConversionError::Overflow { unit } => format!(
                "Time duration '{}' overflows while converting '{}' to microseconds",
                literal, unit
            ),
            TimeConversionError::Cycle { unit } => {
                format!("Time conversion cycle detected while resolving '{}'", unit)
            }
            TimeConversionError::UnknownUnit { unit } => format!(
                "Could not resolve time unit '{}' to a built-in time base while converting '{}'",
                unit, literal
            ),
            TimeConversionError::InvalidConversion { unit } => format!(
                "Time conversion for unit '{}' produced an invalid value while converting '{}'",
                unit, literal
            ),
            TimeConversionError::NonFinite { unit } => format!(
                "Time conversion for unit '{}' produced a non-finite value while converting '{}'",
                unit, literal
            ),
        }
    }

    pub fn new() -> Self {
        TypeChecker {
            symbols: HashMap::new(),
            errors: Vec::new(),
            sensor_read_types: HashMap::new(),
            unit_registry: UnitRegistry::new(),
            custom_time_conversions: HashMap::new(),
            category_base_targets: HashMap::new(),
            unit_base_targets: HashMap::new(),
            externals: HashMap::new(),
            functions: HashMap::new(),
            current_return_type: None,
            current_every_period_micros: None,
            task_blocks_seen: 0,
            inside_loop: false,
        }
    }

    pub fn check_program(&mut self, program: &mut Program) -> Result<(), Vec<TypeError>> {
        self.errors.clear();
        self.symbols.clear();
        self.sensor_read_types.clear();
        self.unit_registry = UnitRegistry::new();
        self.custom_time_conversions.clear();
        self.category_base_targets.clear();
        self.unit_base_targets.clear();
        self.externals.clear();
        self.functions.clear();
        self.current_return_type = None;
        self.current_every_period_micros = None;
        self.task_blocks_seen = 0;
        self.inside_loop = false;
        for stmt in &mut program.statements {
            match stmt {
                TopLevel::SensorDef(def) => {
                    self.check_sensor_def(def);
                }
                TopLevel::OutputDef(def) => {
                    self.validate_c_codegen_identifier(&def.name, def.span);
                    self.symbols.insert(def.name.clone(), Type::Output);
                }
                TopLevel::UnitDef(def) => {
                    self.check_unit_def(def);
                }
                TopLevel::Extern(def) => {
                    self.check_extern_def(def);
                }
                TopLevel::FuncDef(def) => {
                    self.register_func_sig(def);
                    self.check_func_body(def);
                }
                TopLevel::Every(block) => {
                    self.check_every_block(block);
                }
                TopLevel::Task(block) => {
                    self.check_task_block(block);
                }
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    fn register_func_sig(&mut self, def: &FuncDef) {
        if !self.validate_c_codegen_identifier(&def.name, def.span) {
            return;
        }

        let mut param_types = Vec::new();
        for (param_name, type_ann) in &def.params {
            self.validate_c_codegen_identifier(param_name, def.span);
            match parse_type_name(type_ann.as_str()) {
                Some(ty) => param_types.push(ty),
                None => {
                    self.errors.push(TypeError::at(
                        format!(
                            "Unknown type '{}' for parameter '{}' in function '{}'",
                            type_ann, param_name, def.name
                        ),
                        def.span,
                    ));
                    return;
                }
            }
        }

        let ret_type = match parse_type_name(def.ret.as_str()) {
            Some(ty) => ty,
            None => {
                self.errors.push(TypeError::at(
                    format!(
                        "Unknown return type '{}' in function '{}'",
                        def.ret, def.name
                    ),
                    def.span,
                ));
                return;
            }
        };

        if self.functions.contains_key(&def.name) || self.externals.contains_key(&def.name) {
            self.errors.push(TypeError::at(
                format!("Function '{}' is already declared", def.name),
                def.span,
            ));
            return;
        }

        let sig = FunctionSig {
            params: param_types.clone(),
            ret: ret_type.clone(),
        };

        self.functions.insert(def.name.clone(), sig);
        self.symbols.insert(
            def.name.clone(),
            Type::Function(param_types, Box::new(ret_type)),
        );
    }

    fn check_func_body(&mut self, def: &mut FuncDef) {
        let sig = match self.functions.get(&def.name) {
            Some(sig) => sig.clone(),
            None => return,
        };

        let saved_symbols = self.symbols.clone();
        let saved_return_type = self.current_return_type.take();

        for ((param_name, _type_ann), param_type) in def.params.iter().zip(sig.params.iter()) {
            self.symbols.insert(param_name.clone(), param_type.clone());
        }

        self.current_return_type = Some(sig.ret.clone());

        for stmt in &mut def.body {
            self.check_statement(stmt);
        }

        self.symbols = saved_symbols;
        self.current_return_type = saved_return_type;
    }

    fn check_extern_def(&mut self, def: &ExternDef) {
        if !self.validate_c_codegen_path(&def.name, def.span, "extern") {
            return;
        }

        let extern_name = format_extern_name(&def.name);
        let mut param_types = Vec::new();
        for (param_name, type_ann) in &def.params {
            self.validate_c_codegen_identifier(param_name, def.span);
            match parse_type_name(type_ann.as_str()) {
                Some(ty) => param_types.push(ty),
                None => {
                    self.errors.push(TypeError::at(
                        format!(
                            "Unknown type '{}' for parameter '{}' in extern '{}'",
                            type_ann, param_name, extern_name
                        ),
                        def.span,
                    ));
                    return;
                }
            }
        }

        let ret_type = match parse_type_name(def.ret.as_str()) {
            Some(ty) => ty,
            None => {
                self.errors.push(TypeError::at(
                    format!(
                        "Unknown return type '{}' in extern '{}'",
                        def.ret, extern_name
                    ),
                    def.span,
                ));
                return;
            }
        };

        if self.externals.contains_key(&extern_name) {
            self.errors.push(TypeError::at(
                format!("Extern '{}' is already declared", extern_name),
                def.span,
            ));
            return;
        }

        self.externals.insert(
            extern_name,
            FunctionSig {
                params: param_types,
                ret: ret_type,
            },
        );
    }

    fn check_sensor_def(&mut self, def: &SensorDef) {
        if !self.validate_c_codegen_identifier(&def.name, def.span) {
            return;
        }

        self.symbols.insert(def.name.clone(), Type::Sensor);

        self.sensor_read_types.insert(
            def.name.clone(),
            SensorReadType {
                read_type: Type::Int,
            },
        );

        match (&def.category, &def.converter) {
            (None, None) => {}
            (Some(category_name), Some(converter_path)) => {
                self.validate_c_codegen_path(converter_path, def.span, "sensor converter");

                let category = match parse_category(category_name) {
                    Some(category) => category,
                    None => {
                        self.errors.push(TypeError::at(
                            format!(
                                "Invalid sensor category '{}'. Category names must match [a-z][a-z0-9_]*",
                                category_name
                            ),
                            def.span,
                        ));
                        return;
                    }
                };

                if converter_path.is_empty() {
                    self.errors.push(TypeError::at(
                        format!("Sensor '{}' converter path must not be empty", def.name),
                        def.span,
                    ));
                    return;
                }

                let converter_name = format_extern_name(converter_path);
                let converter_sig = self.externals.get(&converter_name).cloned().or_else(|| {
                    if converter_path.len() == 1 {
                        self.functions.get(&converter_name).cloned()
                    } else {
                        None
                    }
                });

                let converter_sig = match converter_sig {
                    Some(sig) => sig,
                    None => {
                        self.errors.push(TypeError::at(
                            format!(
                                "Sensor '{}' converter '{}' must be declared before the sensor",
                                def.name, converter_name
                            ),
                            def.span,
                        ));
                        return;
                    }
                };

                if converter_sig.params.len() != 1 {
                    self.errors.push(TypeError::at(
                        format!(
                            "Sensor '{}' converter '{}' must take exactly one parameter of type int",
                            def.name, converter_name
                        ),
                        def.span,
                    ));
                    return;
                }

                if converter_sig.params[0] != Type::Int {
                    self.errors.push(TypeError::at(
                        format!(
                            "Sensor '{}' converter '{}' parameter must be int, got {}",
                            def.name, converter_name, converter_sig.params[0]
                        ),
                        def.span,
                    ));
                    return;
                }

                let expected_ret = if category == UnitCategory::Time {
                    Type::Int
                } else {
                    Type::Float
                };

                if converter_sig.ret != expected_ret {
                    self.errors.push(TypeError::at(
                        format!(
                            "Sensor '{}' converter '{}' must return {}, got {}",
                            def.name, converter_name, expected_ret, converter_sig.ret
                        ),
                        def.span,
                    ));
                    return;
                }

                self.sensor_read_types.insert(
                    def.name.clone(),
                    SensorReadType {
                        read_type: Type::Unit(category),
                    },
                );
            }
            _ => {
                self.errors.push(TypeError::at(
                    format!(
                        "Sensor '{}' typed declaration must include both category and converter",
                        def.name
                    ),
                    def.span,
                ));
            }
        }
    }

    fn check_unit_def(&mut self, def: &UnitDef) {
        if def.name.is_empty()
            || !def.name.chars().next().unwrap().is_ascii_lowercase()
            || !def
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            self.errors.push(TypeError::at(
                format!(
                    "Unit name '{}' must start with a lowercase letter and be alphanumeric",
                    def.name
                ),
                def.span,
            ));
            return;
        }
        if !self.validate_c_codegen_identifier(&def.name, def.span) {
            return;
        }

        let category = match parse_category(&def.category) {
            Some(cat) => cat,
            None => {
                self.errors.push(TypeError::at(
                    format!(
                        "Invalid unit category '{}'. Category names must match [a-z][a-z0-9_]*",
                        def.category
                    ),
                    def.span,
                ));
                return;
            }
        };

        let mut to_base: Option<ConversionExpr> = None;
        let mut from_base: Option<ConversionExpr> = None;
        let mut to_target: Option<String> = None;
        let mut from_target: Option<String> = None;

        for (key, expr) in &def.conversions {
            self.validate_conversion_expr(expr, def.span);

            if key.starts_with("to_") {
                if to_base.is_some() {
                    self.errors.push(TypeError::at(
                        format!("Unit '{}' has multiple to_* conversions", def.name),
                        def.span,
                    ));
                } else {
                    to_base = Some(expr.clone());
                    let target = key.strip_prefix("to_").unwrap_or_default();
                    if target.is_empty() {
                        self.errors.push(TypeError::at(
                            format!("Unit '{}' has invalid conversion key '{}'", def.name, key),
                            def.span,
                        ));
                    } else {
                        to_target = Some(target.to_string());
                    }
                }
            } else if key.starts_with("from_") {
                if from_base.is_some() {
                    self.errors.push(TypeError::at(
                        format!("Unit '{}' has multiple from_* conversions", def.name),
                        def.span,
                    ));
                } else {
                    from_base = Some(expr.clone());
                    let target = key.strip_prefix("from_").unwrap_or_default();
                    if target.is_empty() {
                        self.errors.push(TypeError::at(
                            format!("Unit '{}' has invalid conversion key '{}'", def.name, key),
                            def.span,
                        ));
                    } else {
                        from_target = Some(target.to_string());
                    }
                }
            } else {
                self.errors.push(TypeError::at(
                    format!("Conversion key '{}' must start with 'to_' or 'from_'", key),
                    def.span,
                ));
            }
        }

        let to_base = match to_base {
            Some(e) => e,
            None => {
                self.errors.push(TypeError::at(
                    format!("Unit '{}' is missing a to_* conversion formula", def.name),
                    def.span,
                ));
                return;
            }
        };

        let from_base = match from_base {
            Some(e) => e,
            None => {
                self.errors.push(TypeError::at(
                    format!("Unit '{}' is missing a from_* conversion formula", def.name),
                    def.span,
                ));
                return;
            }
        };

        let to_target = match to_target {
            Some(target) => target,
            None => {
                self.errors.push(TypeError::at(
                    format!("Unit '{}' is missing a to_* base target", def.name),
                    def.span,
                ));
                return;
            }
        };
        let from_target = match from_target {
            Some(target) => target,
            None => {
                self.errors.push(TypeError::at(
                    format!("Unit '{}' is missing a from_* base target", def.name),
                    def.span,
                ));
                return;
            }
        };
        if to_target != from_target {
            self.errors.push(TypeError::at(
                format!(
                    "Unit '{}' has mismatched conversion targets: to_{} vs from_{}",
                    def.name, to_target, from_target
                ),
                def.span,
            ));
            return;
        }

        let normalized_base_target =
            match self.normalize_base_target(&category, &to_target, def.span) {
                Some(target) => target,
                None => return,
            };
        if !self.enforce_category_base_target(&category, &normalized_base_target, def.span) {
            return;
        }

        let custom_def = CustomUnitDef {
            name: def.name.clone(),
            category: category.clone(),
            to_base,
            from_base,
        };

        if let Err(msg) = self.unit_registry.register(custom_def) {
            self.errors.push(TypeError::at(msg, def.span));
            return;
        }

        self.unit_base_targets
            .insert(def.name.clone(), normalized_base_target.clone());

        if category == UnitCategory::Time {
            self.custom_time_conversions.insert(
                def.name.clone(),
                CustomTimeConversion {
                    target_unit: normalized_base_target,
                    to_target_expr: def
                        .conversions
                        .iter()
                        .find(|(name, _)| name.starts_with("to_"))
                        .map(|(_, expr)| expr.clone())
                        .unwrap_or(ConversionExpr::Val),
                },
            );
        }
    }

    fn validate_conversion_expr(&mut self, expr: &ConversionExpr, span: Option<SourceSpan>) {
        match expr {
            ConversionExpr::Val => {}
            ConversionExpr::Lit(_) => {}
            ConversionExpr::BinaryOp { lhs, op, rhs } => {
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {}
                    _ => {
                        self.errors.push(TypeError::at(
                            format!(
                                "Conversion formulas only support +, -, *, / operators, got {:?}",
                                op
                            ),
                            span,
                        ));
                    }
                }
                self.validate_conversion_expr(lhs, span);
                self.validate_conversion_expr(rhs, span);
            }
            ConversionExpr::Paren(inner) => {
                self.validate_conversion_expr(inner, span);
            }
            ConversionExpr::UnaryNeg(inner) => {
                self.validate_conversion_expr(inner, span);
            }
        }
    }

    fn is_time_unit(&self, unit: &str) -> bool {
        self.unit_registry.categorize(unit) == Some(UnitCategory::Time)
    }

    fn is_autocast_primitive(ty: &Type) -> bool {
        matches!(ty, Type::Int | Type::Float | Type::Bool | Type::String)
    }

    fn can_autocast(from: &Type, to: &Type) -> bool {
        Self::is_autocast_primitive(from) && Self::is_autocast_primitive(to)
    }

    fn is_codegen_safe_for_iterable(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::RangeArray { .. } | ExprKind::Array(_) => true,
            ExprKind::Paren(inner) => Self::is_codegen_safe_for_iterable(inner),
            _ => false,
        }
    }

    fn autocast_expr(expr: &mut Expr, expected: &Type) {
        if let Some(current) = &expr.ty
            && current == expected
        {
            return;
        }

        let original = expr.clone();
        expr.kind = ExprKind::Cast {
            expr: Box::new(original),
            target: expected.clone(),
        };
        expr.ty = Some(expected.clone());
        expr.unit = None;
    }

    fn check_every_block(&mut self, block: &mut EveryBlock) {
        if !self.is_time_unit(&block.interval_unit) {
            self.errors.push(TypeError::at(
                format!(
                    "'every' interval must be a time unit, but '{}' is not a time unit",
                    block.interval_unit
                ),
                block.span,
            ));

            for stmt in &mut block.body {
                self.check_statement(stmt);
            }
            return;
        }

        let period_micros = match self
            .time_value_to_micros_checked(block.interval_value.clone(), &block.interval_unit)
        {
            Ok(period_micros) => Some(period_micros),
            Err(err) => {
                self.errors.push(TypeError::at(
                    format!(
                        "Invalid 'every' interval: {}",
                        Self::time_conversion_error_message(
                            &block.interval_value,
                            &block.interval_unit,
                            &err
                        )
                    ),
                    block.span,
                ));
                None
            }
        };
        let saved = self.current_every_period_micros;
        self.current_every_period_micros = period_micros;

        if let Some(period_micros) = period_micros {
            let total_sleep_micros = self.calculate_max_sleep_micros(&block.body);
            if total_sleep_micros > period_micros {
                self.errors.push(TypeError::at(
                    format!(
                        "Total sleep duration ({}us) exceeds the 'every' period ({}us)",
                        total_sleep_micros, period_micros
                    ),
                    block.span,
                ));
            }
        }

        for stmt in &mut block.body {
            self.check_statement(stmt);
        }

        self.current_every_period_micros = saved;
    }

    fn check_task_block(&mut self, block: &mut TaskBlock) {
        self.task_blocks_seen += 1;
        if self.task_blocks_seen > 1 {
            self.errors.push(TypeError::at(
                "Only one top-level 'task' block is allowed",
                block.span,
            ));
        }

        let saved = self.current_every_period_micros;
        self.current_every_period_micros = None;
        for stmt in &mut block.body {
            self.check_statement(stmt);
        }
        self.current_every_period_micros = saved;
    }

    fn calculate_max_sleep_micros(&self, stmts: &[Statement]) -> i64 {
        let mut total = 0;
        for stmt in stmts {
            let stmt_cost = match stmt {
                Statement::Sleep { value, unit, .. } => {
                    self.time_value_to_micros(value.clone(), unit).unwrap_or(0)
                }
                Statement::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    let t = self.calculate_max_sleep_micros(then_body);
                    let e = if let Some(else_body) = else_body {
                        self.calculate_max_sleep_micros(else_body)
                    } else {
                        0
                    };
                    if t > e { t } else { e }
                }
                Statement::While { body, .. } => self.calculate_max_sleep_micros(body),
                Statement::For { body, .. } => self.calculate_max_sleep_micros(body),
                _ => 0,
            };
            total += stmt_cost;
        }
        total
    }

    fn normalize_base_target(
        &mut self,
        category: &UnitCategory,
        raw_target: &str,
        span: Option<SourceSpan>,
    ) -> Option<String> {
        let normalized = match category {
            UnitCategory::Time => match raw_target {
                "μs" => "us".to_string(),
                "us" | "ms" | "s" | "min" | "h" => raw_target.to_string(),
                _ => {
                    self.errors.push(TypeError::at(
                        format!(
                            "Unsupported time base target '{}'. Use one of: us, ms, s, min, h",
                            raw_target
                        ),
                        span,
                    ));
                    return None;
                }
            },
            UnitCategory::Temperature => match raw_target.to_ascii_lowercase().as_str() {
                "c" | "celsius" => "c".to_string(),
                _ => {
                    self.errors.push(TypeError::at(
                        format!(
                            "Unsupported temperature base target '{}'. Use 'c' or 'celsius'",
                            raw_target
                        ),
                        span,
                    ));
                    return None;
                }
            },
            UnitCategory::Distance => match raw_target.to_ascii_lowercase().as_str() {
                "m" | "meter" | "meters" => "m".to_string(),
                _ => {
                    self.errors.push(TypeError::at(
                        format!(
                            "Unsupported distance base target '{}'. Use 'm', 'meter', or 'meters'",
                            raw_target
                        ),
                        span,
                    ));
                    return None;
                }
            },
            UnitCategory::Voltage => match raw_target.to_ascii_lowercase().as_str() {
                "v" | "volt" | "volts" => "v".to_string(),
                _ => {
                    self.errors.push(TypeError::at(
                        format!(
                            "Unsupported voltage base target '{}'. Use 'v', 'volt', or 'volts'",
                            raw_target
                        ),
                        span,
                    ));
                    return None;
                }
            },
            UnitCategory::Percentage => match raw_target.to_ascii_lowercase().as_str() {
                "%" | "percent" | "percentage" => "%".to_string(),
                _ => {
                    self.errors.push(TypeError::at(
                        format!(
                            "Unsupported percentage base target '{}'. Use '%', 'percent', or 'percentage'",
                            raw_target
                        ),
                        span,
                    ));
                    return None;
                }
            },
            UnitCategory::Frequency => match raw_target.to_ascii_lowercase().as_str() {
                "hz" => "hz".to_string(),
                _ => {
                    self.errors.push(TypeError::at(
                        format!(
                            "Unsupported frequency base target '{}'. Use 'hz'",
                            raw_target
                        ),
                        span,
                    ));
                    return None;
                }
            },
            UnitCategory::Custom(_) => raw_target.to_string(),
        };

        Some(normalized)
    }

    fn enforce_category_base_target(
        &mut self,
        category: &UnitCategory,
        target: &str,
        span: Option<SourceSpan>,
    ) -> bool {
        if let Some(existing) = self.category_base_targets.get(category) {
            if existing != target {
                self.errors.push(TypeError::at(
                    format!(
                        "Category '{}' already uses base target '{}', but '{}' uses '{}'",
                        category, existing, category, target
                    ),
                    span,
                ));
                return false;
            }
            true
        } else {
            self.category_base_targets
                .insert(category.clone(), target.to_string());
            true
        }
    }

    fn category_base_target(&self, category: &UnitCategory) -> Option<String> {
        if let Some(target) = self.category_base_targets.get(category) {
            return Some(target.clone());
        }

        match category {
            UnitCategory::Time => Some("us".to_string()),
            UnitCategory::Temperature => Some("c".to_string()),
            UnitCategory::Distance => Some("m".to_string()),
            UnitCategory::Voltage => Some("v".to_string()),
            UnitCategory::Percentage => Some("%".to_string()),
            UnitCategory::Frequency => Some("hz".to_string()),
            UnitCategory::Custom(_) => None,
        }
    }

    fn normalize_unit_literal(
        &mut self,
        value: Number,
        unit: &str,
        category: &UnitCategory,
        span: Option<SourceSpan>,
    ) -> Option<(Number, String)> {
        let target = match self.category_base_target(category) {
            Some(target) => target,
            None => {
                self.errors.push(TypeError::at(
                    format!("No base target is registered for category '{}'", category),
                    span,
                ));
                return None;
            }
        };

        if let Some(def) = self.unit_registry.custom_units.get(unit) {
            let source_target = self
                .unit_base_targets
                .get(unit)
                .cloned()
                .unwrap_or_else(|| target.clone());
            let converted =
                Self::eval_conversion_expr_with_value(&def.to_base, Self::number_to_f64(value))?;
            if !converted.is_finite() {
                self.errors.push(TypeError::at(
                    format!(
                        "Unit literal conversion for '{}' produced a non-finite value",
                        unit
                    ),
                    span,
                ));
                return None;
            }

            let normalized_value = match category {
                UnitCategory::Time => {
                    let as_target = self.convert_time_between_units(
                        Number::Float(converted),
                        &source_target,
                        &target,
                    )?;
                    Number::Int(as_target)
                }
                _ => {
                    if source_target != target {
                        self.errors.push(TypeError::at(
                            format!(
                                "Unit '{}' converts to '{}', but category '{}' base target is '{}'",
                                unit, source_target, category, target
                            ),
                            span,
                        ));
                        return None;
                    }
                    Number::Float(converted)
                }
            };

            return Some((normalized_value, target));
        }

        self.convert_builtin_unit_to_target(value, unit, category, &target)
            .map(|normalized| (normalized, target))
    }

    fn convert_time_between_units(&self, value: Number, from: &str, to: &str) -> Option<i64> {
        let micros = builtin_time_to_micros(value, from)?;
        match to {
            "us" => Some(micros),
            "ms" => Some((micros as f64 / 1_000.0).round() as i64),
            "s" => Some((micros as f64 / 1_000_000.0).round() as i64),
            "min" => Some((micros as f64 / 60_000_000.0).round() as i64),
            "h" => Some((micros as f64 / 3_600_000_000.0).round() as i64),
            _ => None,
        }
    }

    fn convert_builtin_unit_to_target(
        &self,
        value: Number,
        unit: &str,
        category: &UnitCategory,
        target: &str,
    ) -> Option<Number> {
        match category {
            UnitCategory::Time => {
                let converted = self.convert_time_between_units(value, unit, target)?;
                Some(Number::Int(converted))
            }
            UnitCategory::Temperature => {
                if target != "c" {
                    return None;
                }
                let v = Self::number_to_f64(value);
                let out = match unit.to_ascii_lowercase().as_str() {
                    "c" => v,
                    "f" => (v - 32.0) * 5.0 / 9.0,
                    "k" => v - 273.15,
                    _ => return None,
                };
                Some(Number::Float(out))
            }
            UnitCategory::Distance => {
                if target != "m" {
                    return None;
                }
                let v = Self::number_to_f64(value);
                let out = match unit.to_ascii_lowercase().as_str() {
                    "mm" => v / 1000.0,
                    "cm" => v / 100.0,
                    "m" => v,
                    "km" => v * 1000.0,
                    _ => return None,
                };
                Some(Number::Float(out))
            }
            UnitCategory::Voltage => {
                if target != "v" {
                    return None;
                }
                let v = Self::number_to_f64(value);
                let out = match unit.to_ascii_lowercase().as_str() {
                    "mv" => v / 1000.0,
                    "v" => v,
                    _ => return None,
                };
                Some(Number::Float(out))
            }
            UnitCategory::Percentage => {
                if target != "%" {
                    return None;
                }
                let v = Self::number_to_f64(value);
                if unit == "%" {
                    Some(Number::Float(v))
                } else {
                    None
                }
            }
            UnitCategory::Frequency => {
                if target != "hz" {
                    return None;
                }
                let v = Self::number_to_f64(value);
                let out = match unit.to_ascii_lowercase().as_str() {
                    "hz" => v,
                    "khz" => v * 1000.0,
                    "mhz" => v * 1_000_000.0,
                    _ => return None,
                };
                Some(Number::Float(out))
            }
            UnitCategory::Custom(_) => None,
        }
    }

    fn number_to_f64(value: Number) -> f64 {
        match value {
            Number::Int(i) => i as f64,
            Number::Float(f) => f,
        }
    }

    fn eval_conversion_expr_with_value(expr: &ConversionExpr, value: f64) -> Option<f64> {
        match expr {
            ConversionExpr::Val => Some(value),
            ConversionExpr::Lit(n) => Some(*n),
            ConversionExpr::BinaryOp { lhs, op, rhs } => {
                let left = Self::eval_conversion_expr_with_value(lhs, value)?;
                let right = Self::eval_conversion_expr_with_value(rhs, value)?;
                match op {
                    BinOp::Add => Some(left + right),
                    BinOp::Sub => Some(left - right),
                    BinOp::Mul => Some(left * right),
                    BinOp::Div => {
                        if right == 0.0 {
                            None
                        } else {
                            Some(left / right)
                        }
                    }
                    _ => None,
                }
            }
            ConversionExpr::Paren(inner) => Self::eval_conversion_expr_with_value(inner, value),
            ConversionExpr::UnaryNeg(inner) => {
                Some(-Self::eval_conversion_expr_with_value(inner, value)?)
            }
        }
    }

    fn time_value_to_micros(&self, value: Number, unit: &str) -> Option<i64> {
        self.time_value_to_micros_checked(value, unit).ok()
    }

    fn time_value_to_micros_checked(
        &self,
        value: Number,
        unit: &str,
    ) -> Result<i64, TimeConversionError> {
        let mut seen = HashSet::new();
        self.time_value_to_micros_inner(value, unit, &mut seen)
    }

    fn time_value_to_micros_inner(
        &self,
        value: Number,
        unit: &str,
        seen: &mut HashSet<String>,
    ) -> Result<i64, TimeConversionError> {
        if let Some(micros) = builtin_time_to_micros(value.clone(), unit) {
            return Ok(micros);
        }

        if Self::is_builtin_time_unit(unit) {
            return Err(TimeConversionError::Overflow {
                unit: unit.to_string(),
            });
        }

        if seen.contains(unit) {
            return Err(TimeConversionError::Cycle {
                unit: unit.to_string(),
            });
        }

        let conversion = self.custom_time_conversions.get(unit).ok_or_else(|| {
            TimeConversionError::UnknownUnit {
                unit: unit.to_string(),
            }
        })?;
        seen.insert(unit.to_string());

        let converted_value = Self::eval_conversion_expr_with_value(
            &conversion.to_target_expr,
            Self::number_to_f64(value),
        )
        .ok_or_else(|| TimeConversionError::InvalidConversion {
            unit: unit.to_string(),
        })?;
        if !converted_value.is_finite() {
            seen.remove(unit);
            return Err(TimeConversionError::NonFinite {
                unit: unit.to_string(),
            });
        }

        let result = self.time_value_to_micros_inner(
            Number::Float(converted_value),
            &conversion.target_unit,
            seen,
        );
        seen.remove(unit);
        result
    }

    fn check_statement(&mut self, stmt: &mut Statement) {
        match stmt {
            Statement::Read {
                sensor,
                variable,
                span,
            } => {
                self.validate_c_codegen_identifier(variable, *span);

                let sensor_read_type = match self.symbols.get(sensor) {
                    Some(Type::Sensor) => self
                        .sensor_read_types
                        .get(sensor)
                        .map(|meta| meta.read_type.clone())
                        .unwrap_or(Type::Int),
                    Some(found) => {
                        self.errors.push(TypeError::at(
                            format!("'{}' is not a sensor (found type {})", sensor, found),
                            *span,
                        ));
                        Type::Unknown
                    }
                    None => {
                        self.errors.push(TypeError::at(
                            format!("Undefined sensor '{}'", sensor),
                            *span,
                        ));
                        Type::Unknown
                    }
                };

                if let Type::Unit(read_category) = &sensor_read_type
                    && let Some(Type::Unit(existing_category)) = self.symbols.get(variable)
                    && existing_category != read_category
                {
                    self.errors.push(TypeError::at(
                        format!(
                            "Type mismatch for read target '{}': expected unit category {}, got {}",
                            variable, existing_category, read_category
                        ),
                        *span,
                    ));
                    return;
                }

                if sensor_read_type != Type::Unknown {
                    self.symbols.insert(variable.clone(), sensor_read_type);
                }
            }
            Statement::Write {
                output,
                value,
                span,
            } => {
                match self.symbols.get(output) {
                    Some(Type::Output) => {}
                    Some(found) => {
                        self.errors.push(TypeError::at(
                            format!("'{}' is not an output (found type {})", output, found),
                            *span,
                        ));
                    }
                    None => {
                        self.errors.push(TypeError::at(
                            format!("Undefined output '{}'", output),
                            *span,
                        ));
                    }
                }

                let value_type = self.check_expr(value);
                if value_type != Type::Int {
                    if Self::can_autocast(&value_type, &Type::Int) {
                        Self::autocast_expr(value, &Type::Int);
                    } else {
                        self.errors.push(TypeError::at(
                            format!("Write value must be int, got {}", value_type),
                            value.span.or(*span),
                        ));
                    }
                }
            }
            Statement::If {
                condition,
                then_body,
                else_body,
                span,
            } => {
                let cond_type = self.check_expr(condition);
                if cond_type != Type::Bool {
                    if Self::can_autocast(&cond_type, &Type::Bool) {
                        Self::autocast_expr(condition, &Type::Bool);
                    } else {
                        self.errors.push(TypeError::at(
                            format!("If condition must be bool, got {}", cond_type),
                            condition.span.or(*span),
                        ));
                    }
                }
                for stmt in then_body {
                    self.check_statement(stmt);
                }
                if let Some(else_body) = else_body {
                    for stmt in else_body {
                        self.check_statement(stmt);
                    }
                }
            }
            Statement::While {
                condition,
                body,
                span,
            } => {
                let cond_type = self.check_expr(condition);
                if cond_type != Type::Bool {
                    if Self::can_autocast(&cond_type, &Type::Bool) {
                        Self::autocast_expr(condition, &Type::Bool);
                    } else {
                        self.errors.push(TypeError::at(
                            format!("While condition must be bool, got {}", cond_type),
                            condition.span.or(*span),
                        ));
                    }
                }
                let saved_inside_loop = self.inside_loop;
                self.inside_loop = true;
                for stmt in body {
                    self.check_statement(stmt);
                }
                self.inside_loop = saved_inside_loop;
            }
            Statement::For {
                variable,
                iterable,
                body,
                span,
            } => {
                self.validate_c_codegen_identifier(variable, *span);

                let iter_type = self.check_expr(iterable);
                if let Type::Array(elem_type) = &iter_type {
                    self.symbols.insert(variable.clone(), (**elem_type).clone());
                    if !Self::is_codegen_safe_for_iterable(iterable) {
                        self.errors.push(TypeError::at(
                            "For loop iterable must be a literal array or range for safe C codegen",
                            iterable.span.or(*span),
                        ));
                    }
                } else {
                    self.errors.push(TypeError::at(
                        format!("For loop requires an array iterable, got {}", iter_type),
                        iterable.span.or(*span),
                    ));
                }
                let saved_inside_loop = self.inside_loop;
                self.inside_loop = true;
                for stmt in body {
                    self.check_statement(stmt);
                }
                self.inside_loop = saved_inside_loop;
            }
            Statement::Sleep { value, unit, span } => {
                if self.inside_loop {
                    self.errors.push(TypeError::at(
                        "Sleep statements are not allowed inside loops",
                        *span,
                    ));
                    return;
                }
                if !self.is_time_unit(unit) {
                    self.errors.push(TypeError::at(
                        format!(
                            "'sleep' duration must be a time unit, but '{}' is not a time unit",
                            unit
                        ),
                        *span,
                    ));
                    return;
                }

                if let Err(err) = self.time_value_to_micros_checked(value.clone(), unit) {
                    self.errors.push(TypeError::at(
                        format!(
                            "Invalid sleep duration: {}",
                            Self::time_conversion_error_message(value, unit, &err)
                        ),
                        *span,
                    ));
                }
            }
            Statement::Return {
                value: ret_expr,
                span,
            } => {
                let expected_ret = self.current_return_type.clone();
                match expected_ret {
                    Some(expected_ret) => match ret_expr {
                        Some(expr) => {
                            let actual_type = self.check_expr(expr);
                            if expected_ret == Type::Void {
                                self.errors.push(TypeError::at(
                                    "Cannot return a value from a void function",
                                    expr.span.or(*span),
                                ));
                            } else if actual_type != expected_ret {
                                if Self::can_autocast(&actual_type, &expected_ret) {
                                    Self::autocast_expr(expr, &expected_ret);
                                } else {
                                    self.errors.push(TypeError::at(
                                        format!(
                                            "Return type mismatch: expected {}, got {}",
                                            expected_ret, actual_type
                                        ),
                                        expr.span.or(*span),
                                    ));
                                }
                            }
                        }
                        None => {
                            if expected_ret != Type::Void {
                                self.errors.push(TypeError::at(
                                    format!("Missing return value: expected {}", expected_ret),
                                    *span,
                                ));
                            }
                        }
                    },
                    None => {
                        self.errors.push(TypeError::at(
                            "Return statement outside of a function",
                            *span,
                        ));
                    }
                }
            }
            Statement::Assignment {
                variable,
                value,
                span,
            } => {
                self.validate_c_codegen_identifier(variable, *span);

                let value_type = self.check_expr(value);
                let existing_type = self.symbols.get(variable).cloned();

                match existing_type {
                    Some(Type::Unit(existing_cat)) => {
                        match value_type {
                            Type::Unit(new_cat) if new_cat == existing_cat => {}
                            Type::Int | Type::Float | Type::Unknown => {}
                            other => {
                                self.errors.push(TypeError::at(
                                    format!(
                                        "Type mismatch for '{}': expected unit category {}, got {}",
                                        variable, existing_cat, other
                                    ),
                                    value.span.or(*span),
                                ));
                            }
                        }
                        self.symbols
                            .insert(variable.clone(), Type::Unit(existing_cat));
                    }
                    Some(existing) => {
                        if matches!(value_type, Type::Unit(_)) && !matches!(existing, Type::Unit(_))
                        {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Type mismatch for '{}': expected {}, got {}",
                                    variable, existing, value_type
                                ),
                                value.span.or(*span),
                            ));
                        } else {
                            self.symbols.insert(variable.clone(), value_type);
                        }
                    }
                    None => {
                        self.symbols.insert(variable.clone(), value_type);
                    }
                }
            }
            Statement::Expr(expr) => {
                self.check_expr(expr);
            }
        }
    }

    fn check_expr(&mut self, expr: &mut Expr) -> Type {
        let expr_span = expr.span;
        let ty = match &mut expr.kind {
            ExprKind::IntLit(_) => Type::Int,
            ExprKind::FloatLit(_) => Type::Float,
            ExprKind::BoolLit(_) => Type::Bool,
            ExprKind::StringLit(_) => Type::String,
            ExprKind::UnitLit { value, unit } => {
                let unit_name = unit.clone();
                match self.unit_registry.categorize(&unit_name) {
                    Some(category) => {
                        match self.normalize_unit_literal(
                            value.clone(),
                            &unit_name,
                            &category,
                            expr_span,
                        ) {
                            Some((normalized_value, normalized_unit)) => {
                                *value = normalized_value;
                                *unit = normalized_unit;
                                Type::Unit(category)
                            }
                            None => {
                                self.errors.push(TypeError::at(
                                    format!("Could not normalize unit literal '{}'", unit_name),
                                    expr_span,
                                ));
                                Type::Unknown
                            }
                        }
                    }
                    None => {
                        self.errors.push(TypeError::at(
                            format!("Unknown unit '{}'", unit_name),
                            expr_span,
                        ));
                        Type::Unknown
                    }
                }
            }
            ExprKind::Ident(name) => self.symbols.get(name).cloned().unwrap_or(Type::Unknown),
            ExprKind::BinaryOp { lhs, op, rhs } => {
                let left_type = self.check_expr(lhs);
                let right_type = self.check_expr(rhs);

                match op {
                    BinOp::Add => match (&left_type, &right_type) {
                        (Type::Int, Type::Int) => Type::Int,
                        (Type::Float, Type::Int)
                        | (Type::Int, Type::Float)
                        | (Type::Float, Type::Float) => Type::Float,
                        (Type::String, Type::String) => Type::String,
                        (Type::String, other) if Self::can_autocast(other, &Type::String) => {
                            Self::autocast_expr(rhs, &Type::String);
                            Type::String
                        }
                        (other, Type::String) if Self::can_autocast(other, &Type::String) => {
                            Self::autocast_expr(lhs, &Type::String);
                            Type::String
                        }
                        (Type::Unit(lcat), Type::Unit(rcat)) if lcat == rcat => {
                            Type::Unit(lcat.clone())
                        }
                        (Type::Unit(_), Type::Unit(_)) => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Unit arithmetic requires the same category, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                        (Type::Unit(cat), Type::Int) | (Type::Unit(cat), Type::Float) => {
                            Type::Unit(cat.clone())
                        }
                        (Type::Int, Type::Unit(cat)) | (Type::Float, Type::Unit(cat)) => {
                            Type::Unit(cat.clone())
                        }
                        _ => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Arithmetic operation requires numeric or unit types, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                    },
                    BinOp::Sub => match (&left_type, &right_type) {
                        (Type::Int, Type::Int) => Type::Int,
                        (Type::Float, Type::Int)
                        | (Type::Int, Type::Float)
                        | (Type::Float, Type::Float) => Type::Float,
                        (Type::Unit(lcat), Type::Unit(rcat)) if lcat == rcat => {
                            Type::Unit(lcat.clone())
                        }
                        (Type::Unit(_), Type::Unit(_)) => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Unit arithmetic requires the same category, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                        (Type::Unit(cat), Type::Int) | (Type::Unit(cat), Type::Float) => {
                            Type::Unit(cat.clone())
                        }
                        (Type::Int, Type::Unit(cat)) | (Type::Float, Type::Unit(cat)) => {
                            Type::Unit(cat.clone())
                        }
                        _ => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Arithmetic operation requires numeric or unit types, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                    },
                    BinOp::Mul => match (&left_type, &right_type) {
                        (Type::Int, Type::Int) => Type::Int,
                        (Type::Float, Type::Int)
                        | (Type::Int, Type::Float)
                        | (Type::Float, Type::Float) => Type::Float,
                        (Type::Unit(cat), Type::Int) | (Type::Unit(cat), Type::Float) => {
                            Type::Unit(cat.clone())
                        }
                        (Type::Int, Type::Unit(cat)) | (Type::Float, Type::Unit(cat)) => {
                            Type::Unit(cat.clone())
                        }
                        (Type::Unit(_), Type::Unit(_)) => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Unit multiplication only supports unit-scalar combinations, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                        _ => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Arithmetic operation requires numeric or unit types, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                    },
                    BinOp::Div => match (&left_type, &right_type) {
                        (Type::Int, Type::Int) => Type::Int,
                        (Type::Float, Type::Int)
                        | (Type::Int, Type::Float)
                        | (Type::Float, Type::Float) => Type::Float,
                        (Type::Unit(cat), Type::Int) | (Type::Unit(cat), Type::Float) => {
                            Type::Unit(cat.clone())
                        }
                        (Type::Int, Type::Unit(cat)) | (Type::Float, Type::Unit(cat)) => {
                            Type::Unit(cat.clone())
                        }
                        (Type::Unit(_), Type::Unit(_)) => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Unit division only supports unit-scalar combinations, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                        _ => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Arithmetic operation requires numeric or unit types, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                    },
                    BinOp::Mod => match (&left_type, &right_type) {
                        (Type::Int, Type::Int) => Type::Int,
                        (Type::Float, Type::Int)
                        | (Type::Int, Type::Float)
                        | (Type::Float, Type::Float) => Type::Float,
                        (Type::Unit(cat), Type::Int) | (Type::Unit(cat), Type::Float) => {
                            Type::Unit(cat.clone())
                        }
                        (Type::Int, Type::Unit(cat)) | (Type::Float, Type::Unit(cat)) => {
                            Type::Unit(cat.clone())
                        }
                        (Type::Unit(_), Type::Unit(_)) => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Unit modulo only supports unit-scalar combinations, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                        _ => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Arithmetic operation requires numeric or unit types, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                    },
                    BinOp::Pow => match (&left_type, &right_type) {
                        (Type::Int, Type::Int) => Type::Int,
                        (Type::Float, Type::Int)
                        | (Type::Int, Type::Float)
                        | (Type::Float, Type::Float) => Type::Float,
                        (Type::Unit(cat), Type::Int) | (Type::Unit(cat), Type::Float) => {
                            Type::Unit(cat.clone())
                        }
                        (Type::Int, Type::Unit(cat)) | (Type::Float, Type::Unit(cat)) => {
                            Type::Unit(cat.clone())
                        }
                        (Type::Unit(_), Type::Unit(_)) => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Unit exponentiation only supports unit-scalar combinations, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                        _ => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Arithmetic operation requires numeric or unit types, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                    },
                    BinOp::Eq | BinOp::Neq | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                        match (&left_type, &right_type) {
                            _ if left_type == right_type => Type::Bool,
                            (Type::String, Type::String) => Type::Bool,
                            (Type::Unit(lcat), Type::Unit(rcat)) if lcat == rcat => Type::Bool,
                            (Type::Unit(_), Type::Unit(_)) => {
                                self.errors.push(TypeError::at(
                                    format!(
                                        "Unit comparison requires the same category, got {} and {}",
                                        left_type, right_type
                                    ),
                                    expr_span,
                                ));
                                Type::Unknown
                            }
                            (Type::Unit(_), Type::Int)
                            | (Type::Unit(_), Type::Float)
                            | (Type::Int, Type::Unit(_))
                            | (Type::Float, Type::Unit(_)) => Type::Bool,
                            _ => {
                                self.errors.push(TypeError::at(
                                    format!(
                                        "Comparison requires compatible types, got {} and {}",
                                        left_type, right_type
                                    ),
                                    expr_span,
                                ));
                                Type::Unknown
                            }
                        }
                    }
                    BinOp::And | BinOp::Or => {
                        if left_type == Type::Bool && right_type == Type::Bool {
                            Type::Bool
                        } else if Self::can_autocast(&left_type, &Type::Bool)
                            && Self::can_autocast(&right_type, &Type::Bool)
                        {
                            Self::autocast_expr(lhs, &Type::Bool);
                            Self::autocast_expr(rhs, &Type::Bool);
                            Type::Bool
                        } else {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Logical operation requires bool types, got {} and {}",
                                    left_type, right_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                    }
                }
            }
            ExprKind::UnaryOp { op, expr } => {
                let expr_type = self.check_expr(expr);
                match op {
                    UnOp::Neg => match &expr_type {
                        Type::Int | Type::Float => expr_type,
                        Type::Unit(_) => expr_type,
                        _ => {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Negation requires numeric or unit type, got {}",
                                    expr_type
                                ),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                    },
                    UnOp::Not => {
                        if expr_type == Type::Bool {
                            Type::Bool
                        } else if Self::can_autocast(&expr_type, &Type::Bool) {
                            Self::autocast_expr(expr, &Type::Bool);
                            Type::Bool
                        } else {
                            self.errors.push(TypeError::at(
                                format!("Logical not requires bool type, got {}", expr_type),
                                expr_span,
                            ));
                            Type::Unknown
                        }
                    }
                }
            }
            ExprKind::Cast {
                expr: inner,
                target,
            } => {
                let inner_ty = self.check_expr(inner);
                if Self::can_autocast(&inner_ty, target) {
                    target.clone()
                } else {
                    self.errors.push(TypeError::at(
                        format!("Cannot auto-cast from {} to {}", inner_ty, target),
                        inner.span.or(expr_span),
                    ));
                    Type::Unknown
                }
            }
            ExprKind::RangeArray { .. } => Type::Array(Box::new(Type::Int)),
            ExprKind::Array(elements) => {
                if elements.is_empty() {
                    Type::Array(Box::new(Type::Unknown))
                } else {
                    let first_type = self.check_expr(&mut elements[0]);
                    for elem in elements.iter_mut().skip(1) {
                        let elem_type = self.check_expr(elem);
                        if elem_type != first_type {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Array elements must have the same type, expected {}, got {}",
                                    first_type, elem_type
                                ),
                                elem.span.or(expr_span),
                            ));
                            return Type::Array(Box::new(Type::Unknown));
                        }
                    }
                    Type::Array(Box::new(first_type))
                }
            }
            ExprKind::Index { object, index } => {
                let obj_type = self.check_expr(object);
                let idx_type = self.check_expr(index);

                if idx_type != Type::Int {
                    if Self::can_autocast(&idx_type, &Type::Int) {
                        Self::autocast_expr(index, &Type::Int);
                    } else {
                        self.errors.push(TypeError::at(
                            format!("Array index must be int, got {}", idx_type),
                            index.span.or(expr_span),
                        ));
                    }
                }

                if let Type::Array(elem_type) = obj_type {
                    *elem_type
                } else {
                    self.errors.push(TypeError::at(
                        format!("Cannot index into non-array type {}", obj_type),
                        object.span.or(expr_span),
                    ));
                    Type::Unknown
                }
            }
            ExprKind::Paren(inner) => self.check_expr(inner),
            ExprKind::Call { func, args } => {
                let func_type = self.check_expr(func);
                let maybe_name =
                    self.validate_c_codegen_call_expr_path(func, func.span.or(expr_span));

                match func_type {
                    Type::Function(params, ret) => {
                        if params.len() != args.len() {
                            self.errors.push(TypeError::at(
                                format!(
                                    "Function expects {} arguments, got {}",
                                    params.len(),
                                    args.len()
                                ),
                                func.span.or(expr_span),
                            ));
                        } else {
                            for (i, (param_type, arg)) in
                                params.iter().zip(args.iter_mut()).enumerate()
                            {
                                let arg_type = self.check_expr(arg);
                                if param_type != &arg_type {
                                    if Self::can_autocast(&arg_type, param_type) {
                                        Self::autocast_expr(arg, param_type);
                                    } else {
                                        self.errors.push(TypeError::at(
                                            format!(
                                                "Argument {} has wrong type: expected {}, got {}",
                                                i, param_type, arg_type
                                            ),
                                            arg.span.or(expr_span),
                                        ));
                                    }
                                }
                            }
                        }
                        *ret
                    }
                    Type::Unknown => {
                        if let Some(name) = maybe_name {
                            let sig = self
                                .externals
                                .get(&name)
                                .cloned()
                                .or_else(|| self.functions.get(&name).cloned());
                            if let Some(sig) = sig {
                                if sig.params.len() != args.len() {
                                    self.errors.push(TypeError::at(
                                        format!(
                                            "'{}' expects {} arguments, got {}",
                                            name,
                                            sig.params.len(),
                                            args.len()
                                        ),
                                        func.span.or(expr_span),
                                    ));
                                } else {
                                    for (i, (param_type, arg)) in
                                        sig.params.iter().zip(args.iter_mut()).enumerate()
                                    {
                                        let arg_type = self.check_expr(arg);
                                        if param_type != &arg_type {
                                            if Self::can_autocast(&arg_type, param_type) {
                                                Self::autocast_expr(arg, param_type);
                                            } else {
                                                self.errors.push(TypeError::at(
                                                    format!(
                                                    "'{}' argument {} has wrong type: expected {}, got {}",
                                                    name, i, param_type, arg_type
                                                ),
                                                    arg.span.or(expr_span),
                                                ));
                                            }
                                        }
                                    }
                                }
                                return sig.ret;
                            } else {
                                self.errors.push(TypeError::at(
                                    format!("Undefined function or extern '{}'", name),
                                    func.span.or(expr_span),
                                ));
                            }
                        } else {
                            for arg in args {
                                self.check_expr(arg);
                            }
                        }
                        Type::Unknown
                    }
                    _ => {
                        self.errors.push(TypeError::at(
                            format!("Cannot call non-function type {}", func_type),
                            func.span.or(expr_span),
                        ));
                        Type::Unknown
                    }
                }
            }
            ExprKind::Field { object, field: _ } => {
                let obj_type = self.check_expr(object);

                match &obj_type {
                    Type::Sensor | Type::Output => Type::Pin,
                    _ => Type::Unknown,
                }
            }
        };

        expr.ty = Some(ty.clone());
        if let Type::Unit(cat) = &ty {
            expr.unit = Some(cat.clone());
        } else {
            expr.unit = None;
        }
        ty
    }
}

fn format_extern_name(path: &[String]) -> String {
    path.join("::")
}

fn call_path(expr: &Expr) -> Option<String> {
    match &expr.kind {
        ExprKind::Ident(name) => Some(name.clone()),
        ExprKind::Field { object, field } => {
            let mut path = call_path(object)?;
            path.push_str("::");
            path.push_str(field);
            Some(path)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::{program_parser, token_stream};
    use chumsky::Parser;

    fn parse_and_check(input: &str) -> Result<(), Vec<TypeError>> {
        let lexer = Lexer::new();
        let tokens = lexer.tokenize_spanned(input).unwrap();
        let mut program = program_parser()
            .parse(token_stream(&tokens, input.len()))
            .into_result()
            .unwrap();
        let mut checker = TypeChecker::new();
        checker.check_program(&mut program)
    }

    fn parse_and_check_errors(input: &str) -> Vec<TypeError> {
        let lexer = Lexer::new();
        let tokens = lexer.tokenize_spanned(input).unwrap();
        let mut program = program_parser()
            .parse(token_stream(&tokens, input.len()))
            .into_result()
            .unwrap();
        let mut checker = TypeChecker::new();
        match checker.check_program(&mut program) {
            Ok(()) => vec![],
            Err(errs) => errs,
        }
    }

    fn parse_and_check_program(input: &str) -> Result<Program, Vec<TypeError>> {
        let lexer = Lexer::new();
        let tokens = lexer.tokenize_spanned(input).unwrap();
        let mut program = program_parser()
            .parse(token_stream(&tokens, input.len()))
            .into_result()
            .unwrap();
        let mut checker = TypeChecker::new();
        checker.check_program(&mut program)?;
        Ok(program)
    }

    #[test]
    fn test_valid_unit_def_kelvin() {
        let input = r#"
            unit kelvin : temperature {
                to_celsius: val - 273,
                from_celsius: val + 273
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_valid_unit_def_fahrenheit() {
        let input = r#"
            unit fahrenheit : temperature {
                to_celsius: (val - 32) * 5 / 9,
                from_celsius: val * 9 / 5 + 32
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_valid_unit_def_inches() {
        let input = r#"
            unit inches : distance {
                to_meters: val * 0.0254,
                from_meters: val / 0.0254
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_range_array_for_loop_valid() {
        let input = r#"
            extern consume(x: int) -> void
            every 1s {
                for i in [0..5] {
                    consume(i)
                }
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_for_iterable_identifier_is_rejected_for_c_codegen_safety() {
        let input = r#"
            every 1s {
                arr = [1, 2, 3]
                for i in arr {
                    x = i
                }
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("literal array or range for safe C codegen")
        }));
    }

    #[test]
    fn test_for_iterable_paren_array_is_allowed_for_c_codegen_safety() {
        let input = r#"
            extern consume(x: int) -> void
            every 1s {
                for i in ([1, 2, 3]) {
                    consume(i)
                }
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_empty_range_array_still_types_as_int_array() {
        let input = r#"
            extern consume(x: int) -> void
            every 1s {
                for i in [0..0] {
                    consume(i)
                }
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_custom_unit_used_in_threshold() {
        let input = r#"
            unit kelvin : temperature {
                to_celsius: val - 273,
                from_celsius: val + 273
            }
            sensor temp on A0
            every 1s {
                read temp -> t
                x = 300kelvin
            }
        "#;
        let lexer = Lexer::new();
        let tokens = lexer.tokenize_spanned(input).unwrap();
        let mut program = program_parser()
            .parse(token_stream(&tokens, input.len()))
            .into_result()
            .unwrap();
        let mut checker = TypeChecker::new();
        let result = checker.check_program(&mut program);
        assert!(result.is_ok());
    }

    #[test]
    fn test_custom_unit_used_in_comparison() {
        let input = r#"
            unit kelvin : temperature {
                to_celsius: val - 273,
                from_celsius: val + 273
            }
            sensor temp on A0
            every 1s {
                read temp -> t
                if (t > 300kelvin) {
                    sleep 1s
                }
            }
        "#;
        let lexer = Lexer::new();
        let tokens = lexer.tokenize_spanned(input).unwrap();
        let mut program = program_parser()
            .parse(token_stream(&tokens, input.len()))
            .into_result()
            .unwrap();
        let mut checker = TypeChecker::new();
        let result = checker.check_program(&mut program);
        assert!(result.is_ok());
    }

    #[test]
    fn test_typed_sensor_with_function_converter_is_valid() {
        let input = r#"
            fn convert_temp(raw: int) -> float {
                return 0.0
            }
            sensor temp on A0 : temperature using convert_temp
            every 1s {
                read temp -> t
                if (t > 25c) {
                    sleep 1s
                }
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_typed_sensor_with_namespaced_extern_converter_is_valid() {
        let input = r#"
            extern Sensor::convert_temp(raw: int) -> float
            sensor temp on A0 : temperature using Sensor::convert_temp
            every 1s {
                read temp -> t
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_typed_sensor_with_unknown_converter_fails() {
        let input = r#"
            sensor temp on A0 : temperature using convert_temp
            every 1s {
                read temp -> t
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("converter 'convert_temp' must be declared before the sensor")
        }));
    }

    #[test]
    fn test_typed_sensor_converter_requires_one_int_param() {
        let input = r#"
            fn convert_temp(raw: int, scale: int) -> float {
                return 0.0
            }
            sensor temp on A0 : temperature using convert_temp
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("must take exactly one parameter of type int")
        }));
    }

    #[test]
    fn test_typed_sensor_converter_requires_int_param_type() {
        let input = r#"
            fn convert_temp(raw: float) -> float {
                return raw
            }
            sensor temp on A0 : temperature using convert_temp
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("parameter must be int"))
        );
    }

    #[test]
    fn test_typed_sensor_time_converter_requires_int_return() {
        let input = r#"
            fn convert_tick(raw: int) -> float {
                return 0.0
            }
            sensor tick_sensor on A0 : time using convert_tick
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| e.message.contains("must return int")));
    }

    #[test]
    fn test_typed_sensor_converter_must_be_declared_before_sensor() {
        let input = r#"
            sensor temp on A0 : temperature using convert_temp
            fn convert_temp(raw: int) -> float {
                return 0.0
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("must be declared before the sensor"))
        );
    }

    #[test]
    fn test_typed_sensor_read_rejects_incompatible_existing_unit_variable() {
        let input = r#"
            fn convert_temp(raw: int) -> float {
                return 0.0
            }
            sensor temp on A0 : temperature using convert_temp
            every 1s {
                x = 1s
                read temp -> x
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Type mismatch for read target 'x'"))
        );
    }

    #[test]
    fn test_custom_unit_unknown_fails() {
        let input = r#"
            every 1s {
                x = 300zorkmids
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| e.message.contains("Unknown unit")));
    }

    #[test]
    fn test_custom_category_is_implicit() {
        let input = r#"
            unit psi : pressure {
                to_pa: val * 6894.76,
                from_pa: val / 6894.76
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_custom_units_same_custom_category_are_compatible() {
        let input = r#"
            unit psi : pressure {
                to_pa: val * 6894.76,
                from_pa: val / 6894.76
            }
            unit bar : pressure {
                to_pa: val * 100000.0,
                from_pa: val / 100000.0
            }
            every 1s {
                x = 10psi + 1bar
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_custom_category_base_target_mismatch_rejected() {
        let input = r#"
            unit psi : pressure {
                to_pa: val * 6894.76,
                from_pa: val / 6894.76
            }
            unit bar : pressure {
                to_bar: val,
                from_bar: val
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("already uses base target"))
        );
    }

    #[test]
    fn test_custom_categories_are_incompatible_for_arithmetic() {
        let input = r#"
            unit psi : pressure {
                to_pa: val * 6894.76,
                from_pa: val / 6894.76
            }
            unit lpm : flow {
                to_lps: val / 60,
                from_lps: val * 60
            }
            every 1s {
                x = 10psi + 2lpm
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("requires the same category"))
        );
    }

    #[test]
    fn test_same_category_mixed_units_normalize_to_category_base() {
        let input = r#"
            every 1s {
                x = 1m + 50cm
            }
        "#;
        let program = parse_and_check_program(input).expect("type checking should succeed");

        let TopLevel::Every(block) = &program.statements[0] else {
            panic!("expected every block");
        };
        let Statement::Assignment { value, .. } = &block.body[0] else {
            panic!("expected assignment in every body");
        };
        let ExprKind::BinaryOp { lhs, rhs, .. } = &value.kind else {
            panic!("expected binary operation");
        };

        let ExprKind::UnitLit {
            value: lhs_value,
            unit: lhs_unit,
        } = &lhs.kind
        else {
            panic!("expected normalized left unit literal");
        };
        let ExprKind::UnitLit {
            value: rhs_value,
            unit: rhs_unit,
        } = &rhs.kind
        else {
            panic!("expected normalized right unit literal");
        };

        assert_eq!(lhs_unit, "m");
        assert_eq!(rhs_unit, "m");
        assert!(matches!(lhs_value, Number::Float(v) if (*v - 1.0).abs() < 1e-6));
        assert!(matches!(rhs_value, Number::Float(v) if (*v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn tc_time_rounding_boundary_pass() {
        let input = r#"
            every 0.5ms {
                sleep 500us
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn tc_time_rounding_boundary_fail() {
        let input = r#"
            every 0.5ms {
                sleep 501us
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("exceeds the 'every' period"))
        );
    }

    #[test]
    fn tc_scalar_plus_custom_unit_uses_base_domain() {
        let input = r#"
            unit psi : pressure {
                to_pa: val * 6894.76,
                from_pa: val / 6894.76
            }
            every 1s {
                x = 1psi + 1
            }
        "#;
        let program = parse_and_check_program(input).expect("type checking should succeed");

        let TopLevel::Every(block) = &program.statements[1] else {
            panic!("expected every block");
        };
        let Statement::Assignment { value, .. } = &block.body[0] else {
            panic!("expected assignment");
        };
        assert_eq!(
            value.ty,
            Some(Type::Unit(UnitCategory::Custom("pressure".to_string())))
        );

        let ExprKind::BinaryOp { lhs, rhs, .. } = &value.kind else {
            panic!("expected binary operation");
        };
        let ExprKind::UnitLit {
            value: lhs_value,
            unit: lhs_unit,
        } = &lhs.kind
        else {
            panic!("expected normalized unit literal on lhs");
        };
        assert_eq!(lhs_unit, "pa");
        assert!(matches!(lhs_value, Number::Float(v) if (*v - 6894.76).abs() < 1e-6));
        assert!(matches!(rhs.kind, ExprKind::IntLit(1)));
    }

    #[test]
    fn tc_typed_sensor_time_read_is_unit_time() {
        let input = r#"
            fn convert_tick(raw: int) -> int {
                return raw
            }
            sensor tick_sensor on A0 : time using convert_tick
            every 1s {
                read tick_sensor -> t
                if (t > 100us) {
                    sleep 1s
                }
            }
        "#;
        let program = parse_and_check_program(input).expect("type checking should succeed");

        let TopLevel::Every(block) = &program.statements[2] else {
            panic!("expected every block");
        };
        let Statement::If { condition, .. } = &block.body[1] else {
            panic!("expected if statement");
        };
        let ExprKind::BinaryOp { lhs, .. } = &condition.kind else {
            panic!("expected comparison expression");
        };

        assert_eq!(lhs.ty, Some(Type::Unit(UnitCategory::Time)));
    }

    #[test]
    fn tc_typed_sensor_time_read_into_non_time_unit_var_fails() {
        let input = r#"
            fn convert_tick(raw: int) -> int {
                return raw
            }
            sensor tick_sensor on A0 : time using convert_tick
            every 1s {
                x = 1m
                read tick_sensor -> x
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Type mismatch for read target 'x'"))
        );
    }

    #[test]
    fn tc_custom_time_conversion_cycle_errors_early() {
        let input = r#"
            unit tick : time {
                to_ms: val / 0,
                from_ms: val
            }
            every 1tick { }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| {
            e.message.contains("Invalid 'every' interval")
                && e.message.to_lowercase().contains("invalid value")
        }));
    }

    #[test]
    fn tc_every_time_overflow_errors_early() {
        let input = r#"
            every 999999999999h { }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| {
            e.message.contains("Invalid 'every' interval") && e.message.contains("overflows")
        }));
    }

    #[test]
    fn tc_sleep_time_overflow_errors_early() {
        let input = r#"
            every 1s {
                sleep 999999999999h
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| {
            e.message.contains("Invalid sleep duration") && e.message.contains("overflows")
        }));
    }

    #[test]
    fn test_unit_variable_reassignment_same_category_is_allowed() {
        let input = r#"
            every 1s {
                x = 10cm
                x = 1m
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_unit_variable_reassignment_from_scalar_is_allowed() {
        let input = r#"
            every 1s {
                x = 10cm
                x = 1
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_unit_variable_reassignment_different_category_is_rejected() {
        let input = r#"
            every 1s {
                x = 10cm
                x = 1s
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("expected unit category distance"))
        );
    }

    #[test]
    fn test_error_invalid_category_name_uppercase() {
        let input = r#"
            unit foo : Pressure {
                to_base: val * 9.81,
                from_base: val / 9.81
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("Invalid unit category"));
    }

    #[test]
    fn test_error_invalid_category_name_starts_with_underscore() {
        let input = r#"
            unit foo : _pressure {
                to_base: val * 9.81,
                from_base: val / 9.81
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("Invalid unit category"));
    }

    #[test]
    fn test_error_missing_to_conversion() {
        let input = r#"
            unit kelvin : temperature {
                from_celsius: val + 273
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("missing a to_* conversion"));
    }

    #[test]
    fn test_error_missing_from_conversion() {
        let input = r#"
            unit kelvin : temperature {
                to_celsius: val - 273
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("missing a from_* conversion"));
    }

    #[test]
    fn test_error_duplicate_unit() {
        let input = r#"
            unit kelvin : temperature {
                to_celsius: val - 273,
                from_celsius: val + 273
            }
            unit kelvin : temperature {
                to_celsius: val - 273,
                from_celsius: val + 273
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(!errs.is_empty());
        assert!(errs[0].message.contains("already defined"));
    }

    #[test]
    fn test_error_invalid_conversion_key() {
        let input = r#"
            unit kelvin : temperature {
                convert_celsius: val - 273,
                from_celsius: val + 273
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(!errs.is_empty());
        assert!(
            errs.iter()
                .any(|e| e.message.contains("must start with 'to_' or 'from_'"))
        );
    }

    #[test]
    fn test_error_undefined_unit_literal() {
        let input = r#"
            sensor temp on A0
            every 1s {
                read temp -> t
                if (t > 100foo) {
                    sleep 1s
                }
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Unknown unit 'foo'"))
        );
    }

    #[test]
    fn test_builtin_units_still_work() {
        let input = r#"
            sensor temp on A0
            every 1s {
                read temp -> t
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_unit_def_with_full_program() {
        let input = r#"
            unit kelvin : temperature {
                to_celsius: val - 273,
                from_celsius: val + 273
            }

            sensor temp on A0
            output buzz on D0

            every 1s {
                read temp -> t
                if (t > 30) {
                    sleep 500ms
                }
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_extern_valid_call() {
        let input = r#"
            extern sensor_read(id: u8) -> i16
            sensor temp on A0
            every 1s {
                read temp -> t
                x = sensor_read(t)
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_extern_void_return() {
        let input = r#"
            extern delay_ms(ms: u32) -> void
            every 1s {
                delay_ms(100)
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_namespaced_extern_valid_call() {
        let input = r#"
            extern Serial::println(msg: string) -> void
            every 1s {
                Serial::println("hello")
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_rejects_c_keyword_sensor_identifier_for_codegen() {
        let input = r#"
            sensor auto on A0
            every 1s {
                read auto -> t
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("unsafe identifier for C codegen: 'auto'")
        }));
    }

    #[test]
    fn test_rejects_reserved_identifier_pattern_for_codegen() {
        let input = r#"
            sensor _Bad on A0
            every 1s {
                read _Bad -> x
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("unsafe identifier for C codegen: '_Bad'")
        }));
    }

    #[test]
    fn test_rejects_unsafe_extern_path_segment_for_codegen() {
        let input = r#"
            extern Vendor::__lpc_hook(x: int) -> void
            every 1s {
                Vendor::__lpc_hook(1)
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("unsafe identifier for C codegen: '__lpc_hook'")
        }));
    }

    #[test]
    fn test_extern_wrong_arg_count() {
        let input = r#"
            extern sensor_read(id: u8) -> i16
            every 1s {
                x = sensor_read(1, 2)
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("expects 1 arguments, got 2"))
        );
    }

    #[test]
    fn test_extern_wrong_arg_type() {
        let input = r#"
            extern sensor_read(id: u8) -> i16
            every 1s {
                x = sensor_read(3ms)
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| e.message.contains("wrong type")));
    }

    #[test]
    fn test_extern_arg_autocast_float_to_int() {
        let input = r#"
            extern sensor_read(id: u8) -> i16
            every 1s {
                x = sensor_read(3.14)
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_extern_arg_autocast_int_to_string() {
        let input = r#"
            extern Serial::println(msg: string) -> void
            every 1s {
                Serial::println(42)
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_string_concat_autocast_string_plus_int() {
        let input = r#"
            extern Serial::println(msg: string) -> void
            every 1s {
                Serial::println("A" + 42)
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_string_concat_autocast_int_plus_string() {
        let input = r#"
            extern Serial::println(msg: string) -> void
            every 1s {
                Serial::println(42 + "A")
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_string_concat_does_not_autocast_unit_literal() {
        let input = r#"
            extern Serial::println(msg: string) -> void
            every 1s {
                Serial::println("A" + 42ms)
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("Arithmetic operation requires numeric or unit types")
        }));
    }

    #[test]
    fn test_write_statement_valid() {
        let input = r#"
            output buzz on A0
            every 1s {
                buzz write <- 255
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_write_statement_autocast_float_to_int() {
        let input = r#"
            output buzz on A0
            every 1s {
                buzz write <- 3.14
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_write_statement_rejects_unit_literal() {
        let input = r#"
            output buzz on A0
            every 1s {
                buzz write <- 3ms
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Write value must be int"))
        );
    }

    #[test]
    fn test_write_statement_requires_output() {
        let input = r#"
            sensor temp on A0
            every 1s {
                temp write <- 255
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| e.message.contains("is not an output")));
    }

    #[test]
    fn test_extern_duplicate_declaration() {
        let input = r#"
            extern sensor_read(id: u8) -> i16
            extern sensor_read(id: u8) -> i16
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| e.message.contains("already declared")));
    }

    #[test]
    fn test_extern_unknown_param_type() {
        let input = r#"
            extern foo(x: banana) -> void
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Unknown type 'banana'"))
        );
    }

    #[test]
    fn test_extern_unknown_return_type() {
        let input = r#"
            extern foo(x: u8) -> banana
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Unknown return type 'banana'"))
        );
    }

    #[test]
    fn test_extern_return_type_used() {
        let input = r#"
            extern get_value() -> int
            every 1s {
                x = get_value()
                y = x + 1
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_extern_with_full_program() {
        let input = r#"
            extern sensor_read(id: u8) -> i16
            extern delay_ms(ms: u32) -> void

            sensor temp on A0
            output buzz on D0

            every 1s {
                read temp -> t
                x = sensor_read(t)
                if (x > 30) {
                    delay_ms(100)
                }
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_func_def_valid() {
        let input = r#"
            fn add(a: int, b: int) -> int {
                return a + b
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_func_def_void() {
        let input = r#"
            fn do_nothing() -> void {
                return
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_func_call_valid() {
        let input = r#"
            fn double_it(x: int) -> int {
                return x * 2
            }
            every 1s {
                y = double_it(5)
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_func_call_return_type_used() {
        let input = r#"
            fn get_value() -> int {
                return 42
            }
            every 1s {
                x = get_value()
                y = x + 1
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_func_call_wrong_arg_count() {
        let input = r#"
            fn add(a: int, b: int) -> int {
                return a + b
            }
            every 1s {
                x = add(1)
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("expects 2 arguments, got 1"))
        );
    }

    #[test]
    fn test_func_call_wrong_arg_type() {
        let input = r#"
            fn square(x: int) -> int {
                return x * x
            }
            every 1s {
                x = square(3ms)
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| e.message.contains("wrong type")));
    }

    #[test]
    fn test_func_call_autocast_float_to_int() {
        let input = r#"
            fn square(x: int) -> int {
                return x * x
            }
            every 1s {
                x = square(3.14)
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_func_return_type_mismatch() {
        let input = r#"
            fn bad() -> int {
                return 500ms
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Return type mismatch"))
        );
    }

    #[test]
    fn test_func_return_autocast_bool_to_int() {
        let input = r#"
            fn ok() -> int {
                return true
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_if_condition_autocast_int_to_bool() {
        let input = r#"
            every 1s {
                if (1) {
                    x = 1
                }
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_func_void_return_with_value() {
        let input = r#"
            fn bad() -> void {
                return 42
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("Cannot return a value from a void function")
        }));
    }

    #[test]
    fn test_func_missing_return_value() {
        let input = r#"
            fn bad() -> int {
                return
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Missing return value"))
        );
    }

    #[test]
    fn test_func_unknown_param_type() {
        let input = r#"
            fn bad(x: banana) -> void {
                return
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Unknown type 'banana'"))
        );
    }

    #[test]
    fn test_func_unknown_return_type() {
        let input = r#"
            fn bad(x: int) -> banana {
                return x
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Unknown return type 'banana'"))
        );
    }

    #[test]
    fn test_func_duplicate_declaration() {
        let input = r#"
            fn foo() -> void { return }
            fn foo() -> void { return }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| e.message.contains("already declared")));
    }

    #[test]
    fn test_func_conflicts_with_extern() {
        let input = r#"
            extern foo() -> void
            fn foo() -> void { return }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(errs.iter().any(|e| e.message.contains("already declared")));
    }

    #[test]
    fn test_func_forward_reference() {
        let input = r#"
            every 1s {
                x = add(1, 2)
            }
            fn add(a: int, b: int) -> int {
                return a + b
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(!errs.is_empty(), "expected forward-reference error");
    }

    #[test]
    fn test_extern_forward_reference_rejected() {
        let input = r#"
            every 1s {
                x = sensor_read(1)
            }
            extern sensor_read(id: u8) -> i16
        "#;
        let errs = parse_and_check_errors(input);
        assert!(!errs.is_empty(), "expected extern forward-reference error");
    }

    #[test]
    fn test_sensor_forward_reference_rejected() {
        let input = r#"
            every 1s {
                read temp -> t
            }
            sensor temp on A0
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Undefined sensor 'temp'"))
        );
    }

    #[test]
    fn test_unit_forward_reference_rejected() {
        let input = r#"
            every 1s {
                x = 100tick
            }
            unit tick : time {
                to_ms: val * 10,
                from_ms: val / 10
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("Unknown unit 'tick'"))
        );
    }

    #[test]
    fn test_unit_add_same_category() {
        let input = r#"
            every 1s {
                x = 1s + 2s
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_unit_sub_same_category() {
        let input = r#"
            every 1s {
                x = 5s - 2s
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_unit_add_cross_category_rejected() {
        let input = r#"
            every 1s {
                x = 5v + 30c
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("requires the same category"))
        );
    }

    #[test]
    fn test_unit_sub_cross_category_rejected() {
        let input = r#"
            every 1s {
                x = 10m - 3s
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("requires the same category"))
        );
    }

    #[test]
    fn test_unit_mul_scalar_right() {
        let input = r#"
            every 1s {
                x = 5s * 3
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_unit_mul_scalar_left() {
        let input = r#"
            every 1s {
                x = 3 * 5s
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_unit_mul_float_scalar() {
        let input = r#"
            every 1s {
                x = 5s * 2.5
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_unit_mul_unit_rejected() {
        let input = r#"
            every 1s {
                x = 5s * 3s
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("unit-scalar combinations"))
        );
    }

    #[test]
    fn test_unit_div_scalar() {
        let input = r#"
            every 1s {
                x = 10s / 2
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_unit_div_same_category_rejected() {
        let input = r#"
            every 1s {
                x = 10s / 5s
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("unit-scalar combinations"))
        );
    }

    #[test]
    fn test_unit_div_cross_category_rejected() {
        let input = r#"
            every 1s {
                x = 10v / 5s
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("unit-scalar combinations"))
        );
    }

    #[test]
    fn test_unit_mod_units_rejected() {
        let input = r#"
            every 1s {
                x = 10s % 3s
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("unit-scalar combinations"))
        );
    }

    #[test]
    fn test_unit_compare_same_category() {
        let input = r#"
            every 1s {
                if (5s > 3s) {
                    sleep 1s
                }
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_unit_compare_cross_category_rejected() {
        let input = r#"
            every 1s {
                if (5v > 3c) {
                    sleep 1s
                }
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("requires the same category"))
        );
    }

    #[test]
    fn test_unit_negation() {
        let input = r#"
            every 1s {
                x = -5s
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_unit_add_to_int() {
        let input = r#"
            every 1s {
                x = 5s + 3
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_unit_compare_to_int() {
        let input = r#"
            every 1s {
                if (5s > 3) {
                    sleep 1s
                }
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_custom_unit_arithmetic() {
        let input = r#"
            unit kelvin : temperature {
                to_celsius: val - 273,
                from_celsius: val + 273
            }
            every 1s {
                x = 300kelvin + 50kelvin
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_custom_unit_cross_builtin_rejected() {
        let input = r#"
            unit kelvin : temperature {
                to_celsius: val - 273,
                from_celsius: val + 273
            }
            every 1s {
                x = 300kelvin + 5s
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("requires the same category"))
        );
    }

    #[test]
    fn test_every_time_unit_valid() {
        assert!(parse_and_check("every 1s { }").is_ok());
        assert!(parse_and_check("every 500ms { }").is_ok());
        assert!(parse_and_check("every 2min { }").is_ok());
        assert!(parse_and_check("every 1h { }").is_ok());
    }

    #[test]
    fn test_every_non_time_unit_rejected() {
        let errs = parse_and_check_errors("every 30c { }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("'every' interval")
                    && e.message.contains("not a time unit")),
            "expected time-unit error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_every_distance_unit_rejected() {
        let errs = parse_and_check_errors("every 10m { }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("'every' interval")
                    && e.message.contains("not a time unit")),
            "expected time-unit error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_every_custom_time_unit_valid() {
        let input = r#"
            unit tick : time {
                to_ms: val * 10,
                from_ms: val / 10
            }
            every 100tick { }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_every_custom_non_time_unit_rejected() {
        let input = r#"
            unit kelvin : temperature {
                to_celsius: val - 273,
                from_celsius: val + 273
            }
            every 300kelvin { }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("'every' interval")
                    && e.message.contains("not a time unit")),
            "expected time-unit error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_every_custom_category_unit_rejected_when_not_time() {
        let input = r#"
            unit psi : pressure {
                to_pa: val * 6894.76,
                from_pa: val / 6894.76
            }
            every 10psi { }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("'every' interval")
                    && e.message.contains("not a time unit")),
            "expected time-unit error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_sleep_time_unit_valid() {
        assert!(parse_and_check("every 1s { sleep 500ms }").is_ok());
        assert!(parse_and_check("every 1s { sleep 1s }").is_ok());
        assert!(parse_and_check("every 1s { sleep 200ms }").is_ok());
        assert!(parse_and_check("every 1min { sleep 30s }").is_ok());
    }

    #[test]
    fn test_sleep_non_time_unit_rejected() {
        let errs = parse_and_check_errors("every 1s { sleep 30c }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("'sleep' duration")
                    && e.message.contains("not a time unit")),
            "expected time-unit error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_sleep_exceeds_every_period_rejected() {
        let errs = parse_and_check_errors("every 1s { sleep 5s }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("exceeds the 'every' period")),
            "expected period-exceeded error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_sleep_equals_every_period_allowed() {
        assert!(parse_and_check("every 1s { sleep 1s }").is_ok());
        assert!(parse_and_check("every 1s { sleep 1000ms }").is_ok());
    }

    #[test]
    fn test_sleep_exceeds_period_different_units() {
        let errs = parse_and_check_errors("every 1s { sleep 2000ms }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("exceeds the 'every' period")),
            "expected period-exceeded error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_sleep_exceeds_period_in_nested_if() {
        let input = "every 1s { if (1 > 0) { sleep 5s } }";
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("exceeds the 'every' period")),
            "expected period-exceeded error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_sleep_outside_every_no_period_check() {
        let input = r#"
            fn wait() -> void {
                sleep 999s
                return
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_sleep_custom_time_unit_valid() {
        let input = r#"
            unit tick : time {
                to_ms: val * 10,
                from_ms: val / 10
            }
            every 1s { sleep 50tick }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_sleep_custom_time_unit_exceeds_custom_every_rejected() {
        let input = r#"
            unit tick : time {
                to_ms: val * 10,
                from_ms: val / 10
            }
            every 100tick {
                sleep 200tick
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("exceeds the 'every' period")),
            "expected period-exceeded error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_sleep_builtin_time_exceeds_custom_every_rejected() {
        let input = r#"
            unit tick : time {
                to_ms: val * 10,
                from_ms: val / 10
            }
            every 100tick {
                sleep 2s
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("exceeds the 'every' period")),
            "expected period-exceeded error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_sleep_custom_non_time_unit_rejected() {
        let input = r#"
            unit kelvin : temperature {
                to_celsius: val - 273,
                from_celsius: val + 273
            }
            every 1s { sleep 300kelvin }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("'sleep' duration")
                    && e.message.contains("not a time unit")),
            "expected time-unit error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_sleep_custom_category_unit_rejected_when_not_time() {
        let input = r#"
            unit psi : pressure {
                to_pa: val * 6894.76,
                from_pa: val / 6894.76
            }
            every 1s { sleep 5psi }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("'sleep' duration")
                    && e.message.contains("not a time unit")),
            "expected time-unit error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_pin_type_support() {
        let input = r#"
            extern tone(p: Pin, freq: u32) -> void
            output buzz on D0
            every 1s {
                tone(buzz.pin, 1000)
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_every_total_sleep_exceeded() {
        let input = r#"
            every 5s {
                sleep 3s
                sleep 3s
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter().any(
                |e| e.message.contains("Total sleep duration") && e.message.contains("exceeds")
            ),
            "expected total sleep exceeded error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_every_total_sleep_ok_branches() {
        let input = r#"
            every 5s {
                if (1 > 0) {
                    sleep 3s
                } else {
                    sleep 3s
                }
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_every_total_sleep_exceeded_branch_sequence() {
        let input = r#"
            every 5s {
                if (1 > 0) {
                    sleep 3s
                }
                sleep 3s
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter().any(
                |e| e.message.contains("Total sleep duration") && e.message.contains("exceeds")
            ),
            "expected total sleep exceeded error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_every_total_sleep_loop() {
        let input = r#"
            every 5s {
                while (1 > 0) {
                    sleep 6s
                }
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter().any(
                |e| e.message.contains("Total sleep duration") && e.message.contains("exceeds")
            ),
            "expected total sleep exceeded error, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_task_sleep_allowed_without_every_period_cap() {
        let input = r#"
            task {
                sleep 999s
            }
        "#;
        assert!(parse_and_check(input).is_ok());
    }

    #[test]
    fn test_task_sleep_inside_loop_rejected() {
        let input = r#"
            task {
                while (1 > 0) {
                    sleep 1s
                }
            }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter().any(|e| e
                .message
                .contains("Sleep statements are not allowed inside loops")),
            "expected loop sleep rejection, got: {:?}",
            errs
        );
    }

    #[test]
    fn test_multiple_task_blocks_rejected() {
        let input = r#"
            task { x = 1 }
            task { x = 2 }
        "#;
        let errs = parse_and_check_errors(input);
        assert!(
            errs.iter().any(|e| e
                .message
                .contains("Only one top-level 'task' block is allowed")),
            "expected multiple task block rejection, got: {:?}",
            errs
        );
    }
}
