use crate::config::CompilerConfig;
use crate::codegen::{CodegenError, CodegenResult};
use crate::diagnostics::SourceSpan;
use crate::task_ir::{
    EnergyPolicy, IrBinOp, IrDefinition, IrExpr, IrExprKind, IrExtern, IrFunction, IrProgram,
    IrStmt, IrTask, IrType, IrUnOp, OverrunPolicy, PowerState, SchedulerModel, SleepFallback,
    WakeSource,
};
use crate::types::UnitCategory;
use std::collections::{HashMap, HashSet};

const SCALAR_STRING_BUFFER_SIZE: usize = 32;
const CONCAT_STRING_BUFFER_SIZE: usize = 128;

pub fn format_includes(config: &CompilerConfig, default_includes: &[&str]) -> String {
    let mut code = String::new();
    let mut seen = std::collections::HashSet::new();

    for inc in default_includes {
        if seen.insert(*inc) {
            code.push_str(&format!("#include {}\n", inc));
        }
    }
    for inc in &config.c_includes {
        if seen.insert(inc.as_str()) {
            code.push_str(&format!("#include {}\n", inc));
        }
    }
    code.push('\n');
    code
}

fn format_raw_lines(lines: &[String], indent: &str) -> String {
    let mut code = String::new();

    for block in lines {
        if block.is_empty() {
            code.push('\n');
            continue;
        }

        for line in block.lines() {
            if line.is_empty() {
                code.push('\n');
            } else {
                code.push_str(indent);
                code.push_str(line);
                code.push('\n');
            }
        }
    }

    code
}

pub fn format_prelude(config: &CompilerConfig) -> String {
    if config.c_prelude.is_empty() {
        return String::new();
    }

    let mut code = format_raw_lines(&config.c_prelude, "");
    code.push('\n');
    code
}

fn power_state_name(state: &PowerState) -> &'static str {
    match state {
        PowerState::Idle => "idle",
    }
}

fn wake_source_name(source: &WakeSource) -> &'static str {
    match source {
        WakeSource::Tick => "tick",
        WakeSource::Timer => "timer",
    }
}

fn overrun_policy_name(policy: &OverrunPolicy) -> &'static str {
    match policy {
        OverrunPolicy::Skip => "skip",
    }
}

fn sleep_fallback_name(fallback: &SleepFallback) -> &'static str {
    match fallback {
        SleepFallback::UseActiveDelay => "active-delay",
    }
}

fn scheduler_model_name(model: &SchedulerModel) -> &'static str {
    match model {
        SchedulerModel::TickDriven => "tick-driven",
    }
}

fn source_span_comment(span: Option<SourceSpan>, indent: &str) -> String {
    match span {
        Some(span) => format!(
            "{}/* source span: {}..{} */\n",
            indent, span.start, span.end
        ),
        None => String::new(),
    }
}

fn stmt_source_span(stmt: &IrStmt) -> Option<SourceSpan> {
    match stmt {
        IrStmt::Read { source_span, .. }
        | IrStmt::Write { source_span, .. }
        | IrStmt::Sleep { source_span, .. }
        | IrStmt::If { source_span, .. }
        | IrStmt::While { source_span, .. }
        | IrStmt::For { source_span, .. }
        | IrStmt::Assign { source_span, .. }
        | IrStmt::Return { source_span, .. }
        | IrStmt::PeriodicBlock { source_span, .. } => *source_span,
        IrStmt::Expr(expr) => expr.source_span,
    }
}

fn format_wake_sources(sources: &[WakeSource]) -> String {
    if sources.is_empty() {
        "none".to_string()
    } else {
        sources
            .iter()
            .map(wake_source_name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub fn format_autocast_helpers() -> String {
    r#"
static inline void __lpc_copy_cstr(char* dest, size_t dest_size, const char* value) {
    size_t i = 0u;
    const char* src = (value == NULL) ? "" : value;

    if (dest_size == 0u) {
        return;
    }

    while ((i + 1u) < dest_size && src[i] != '\0') {
        dest[i] = src[i];
        ++i;
    }
    dest[i] = '\0';
}

static inline void __lpc_append_cstr(char* dest, size_t dest_size, const char* value) {
    size_t len = 0u;
    size_t i = 0u;
    const char* src = (value == NULL) ? "" : value;

    if (dest_size == 0u) {
        return;
    }

    while (len < dest_size && dest[len] != '\0') {
        ++len;
    }

    if (len >= dest_size) {
        dest[dest_size - 1u] = '\0';
        return;
    }

    while ((len + i + 1u) < dest_size && src[i] != '\0') {
        dest[len + i] = src[i];
        ++i;
    }
    dest[len + i] = '\0';
}

static inline int __lpc_to_int_from_string(const char* value) {
    if (value == NULL) {
        return 0;
    }
    return (int)strtol(value, NULL, 10);
}

static inline float __lpc_to_float_from_string(const char* value) {
    if (value == NULL) {
        return 0.0f;
    }
    return (float)atof(value);
}

static inline bool __lpc_to_bool_from_string(const char* value) {
    if (value == NULL || value[0] == '\0') {
        return false;
    }
    return true;
}

static inline void __lpc_format_int_to_buffer(char* dest, size_t dest_size, int value) {
    if (dest_size == 0u) {
        return;
    }
    snprintf(dest, dest_size, "%d", value);
    dest[dest_size - 1u] = '\0';
}

static inline void __lpc_format_float_to_buffer(char* dest, size_t dest_size, float value) {
    if (dest_size == 0u) {
        return;
    }
#if defined(__AVR__)
    dtostrf((double)value, 1, 6, dest);
#else
    snprintf(dest, dest_size, "%.6g", (double)value);
#endif
    dest[dest_size - 1u] = '\0';
}

static inline void __lpc_format_bool_to_buffer(char* dest, size_t dest_size, bool value) {
    __lpc_copy_cstr(dest, dest_size, value ? "true" : "false");
}

"#
    .to_string()
}

fn escape_c_string_literal(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\0' => escaped.push_str("\\0"),
            c if c.is_control() => {
                let code = c as u32;
                if code <= 0xFF {
                    escaped.push_str(&format!("\\x{:02X}", code));
                } else {
                    escaped.push(c);
                }
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn format_float_literal(value: f64) -> String {
    if value.is_nan() {
        "0.0f".to_string()
    } else if value.is_infinite() {
        if value.is_sign_negative() {
            "-3.4028235e+38f".to_string()
        } else {
            "3.4028235e+38f".to_string()
        }
    } else {
        let mut rendered = format!("{}", value);
        if !rendered.contains('.') && !rendered.contains('e') && !rendered.contains('E') {
            rendered.push_str(".0");
        }
        rendered.push('f');
        rendered
    }
}

fn format_number_literal(value: &crate::ast::Number) -> String {
    match value {
        crate::ast::Number::Int(i) => i.to_string(),
        crate::ast::Number::Float(f) => format_float_literal(*f),
    }
}

#[derive(Debug, Clone, Default)]
struct GeneratedExpr {
    prelude: String,
    expr: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BufferStorage {
    Automatic,
    Static,
}

#[derive(Debug, Default)]
struct BodyCodegenState {
    loop_counter: usize,
    temp_counter: usize,
}

fn next_temp_name(state: &mut BodyCodegenState, prefix: &str) -> String {
    let temp_id = state.temp_counter;
    state.temp_counter += 1;
    format!("__lpc_{}_{}", prefix, temp_id)
}

fn declare_buffer(
    indent: &str,
    name: &str,
    size: usize,
    storage: BufferStorage,
) -> String {
    let storage_prefix = if matches!(storage, BufferStorage::Static) {
        "static "
    } else {
        ""
    };
    format!("{indent}{storage_prefix}char {name}[{size}];\n")
}

fn flatten_string_concat<'a>(expr: &'a IrExpr, parts: &mut Vec<&'a IrExpr>) {
    if let IrExprKind::BinaryOp { lhs, op, rhs } = &expr.kind
        && matches!(op, IrBinOp::Add) && expr.ty == IrType::String
    {
        flatten_string_concat(lhs, parts);
        flatten_string_concat(rhs, parts);
        return;
    }
    parts.push(expr);
}

fn emit_string_cast(
    expr: &IrExpr,
    inner: GeneratedExpr,
    indent: &str,
    state: &mut BodyCodegenState,
    storage: BufferStorage,
) -> CodegenResult<GeneratedExpr> {
    let helper = match &expr.kind {
        IrExprKind::Cast { expr, target } if *target == IrType::String => match &expr.ty {
            IrType::Int => "__lpc_format_int_to_buffer",
            IrType::Float => "__lpc_format_float_to_buffer",
            IrType::Bool => "__lpc_format_bool_to_buffer",
            IrType::String => {
                return Ok(inner);
            }
            other => {
                return Err(CodegenError::unsupported_ir(
                    format!("string autocast from {} is not supported in C codegen", other),
                    expr.source_span,
                ));
            }
        },
        _ => {
            return Err(CodegenError::unsupported_ir(
                "expected string cast during string emission",
                expr.source_span,
            ));
        }
    };

    let buffer_name = next_temp_name(state, "string_scalar");
    let mut prelude = inner.prelude;
    prelude.push_str(&declare_buffer(
        indent,
        &buffer_name,
        SCALAR_STRING_BUFFER_SIZE,
        storage,
    ));
    prelude.push_str(&format!(
        "{indent}{helper}({buffer_name}, sizeof({buffer_name}), {});\n",
        inner.expr
    ));

    Ok(GeneratedExpr {
        prelude,
        expr: buffer_name,
    })
}

fn emit_string_concat(
    expr: &IrExpr,
    indent: &str,
    state: &mut BodyCodegenState,
    storage: BufferStorage,
) -> CodegenResult<GeneratedExpr> {
    let mut parts = Vec::new();
    flatten_string_concat(expr, &mut parts);

    let buffer_name = next_temp_name(state, "string_concat");
    let mut prelude = declare_buffer(
        indent,
        &buffer_name,
        CONCAT_STRING_BUFFER_SIZE,
        storage,
    );
    prelude.push_str(&format!("{indent}{buffer_name}[0] = '\\0';\n"));

    for part in parts {
        if part.ty != IrType::String {
            return Err(CodegenError::unsupported_ir(
                "string concatenation part was not lowered to string",
                part.source_span,
            ));
        }
        let rendered = emit_expr(part, indent, state, BufferStorage::Automatic)?;
        prelude.push_str(&rendered.prelude);
        prelude.push_str(&format!(
            "{indent}__lpc_append_cstr({buffer_name}, sizeof({buffer_name}), {});\n",
            rendered.expr
        ));
    }

    Ok(GeneratedExpr {
        prelude,
        expr: buffer_name,
    })
}

fn emit_expr(
    expr: &IrExpr,
    indent: &str,
    state: &mut BodyCodegenState,
    string_storage: BufferStorage,
) -> CodegenResult<GeneratedExpr> {
    match &expr.kind {
        IrExprKind::IntLit(v) => Ok(GeneratedExpr {
            prelude: String::new(),
            expr: format!("{v}"),
        }),
        IrExprKind::FloatLit(v) => Ok(GeneratedExpr {
            prelude: String::new(),
            expr: format_float_literal(*v),
        }),
        IrExprKind::BoolLit(v) => Ok(GeneratedExpr {
            prelude: String::new(),
            expr: if *v {
                "true".to_string()
            } else {
                "false".to_string()
            },
        }),
        IrExprKind::StringLit(s) => Ok(GeneratedExpr {
            prelude: String::new(),
            expr: format!("\"{}\"", escape_c_string_literal(s)),
        }),
        IrExprKind::UnitLit { value, .. } => Ok(GeneratedExpr {
            prelude: String::new(),
            expr: format_number_literal(value),
        }),
        IrExprKind::Ident(id) => Ok(GeneratedExpr {
            prelude: String::new(),
            expr: id.clone(),
        }),
        IrExprKind::BinaryOp { lhs, op, rhs } => {
            if matches!(op, IrBinOp::Add) && expr.ty == IrType::String {
                return emit_string_concat(expr, indent, state, string_storage);
            }

            let lhs = emit_expr(lhs, indent, state, BufferStorage::Automatic)?;
            let rhs = emit_expr(rhs, indent, state, BufferStorage::Automatic)?;
            let op_str = match op {
                IrBinOp::Add => "+",
                IrBinOp::Sub => "-",
                IrBinOp::Mul => "*",
                IrBinOp::Div => "/",
                IrBinOp::Mod => "%",
                IrBinOp::Eq => "==",
                IrBinOp::Neq => "!=",
                IrBinOp::Lt => "<",
                IrBinOp::Gt => ">",
                IrBinOp::Le => "<=",
                IrBinOp::Ge => ">=",
                IrBinOp::And => "&&",
                IrBinOp::Or => "||",
                IrBinOp::Pow => {
                    return Err(CodegenError::unsupported_ir(
                        "power operator is not supported in C codegen",
                        expr.source_span,
                    ));
                }
            };
            Ok(GeneratedExpr {
                prelude: format!("{}{}", lhs.prelude, rhs.prelude),
                expr: format!("({} {} {})", lhs.expr, op_str, rhs.expr),
            })
        }
        IrExprKind::UnaryOp { op, expr: inner } => {
            let inner = emit_expr(inner, indent, state, BufferStorage::Automatic)?;
            let op_str = match op {
                IrUnOp::Neg => "-",
                IrUnOp::Not => "!",
            };
            Ok(GeneratedExpr {
                prelude: inner.prelude,
                expr: format!("{op_str}({})", inner.expr),
            })
        }
        IrExprKind::Cast {
            expr: inner_expr,
            target,
        } => {
            let inner = emit_expr(inner_expr, indent, state, BufferStorage::Automatic)?;
            match target {
                IrType::Int => Ok(GeneratedExpr {
                    prelude: inner.prelude,
                    expr: match &inner_expr.ty {
                        IrType::String => format!("__lpc_to_int_from_string({})", inner.expr),
                        _ => format!("((int)({}))", inner.expr),
                    },
                }),
                IrType::Float => Ok(GeneratedExpr {
                    prelude: inner.prelude,
                    expr: match &inner_expr.ty {
                        IrType::String => format!("__lpc_to_float_from_string({})", inner.expr),
                        _ => format!("((float)({}))", inner.expr),
                    },
                }),
                IrType::Bool => Ok(GeneratedExpr {
                    prelude: inner.prelude,
                    expr: match &inner_expr.ty {
                        IrType::String => format!("__lpc_to_bool_from_string({})", inner.expr),
                        _ => format!("(({}) != 0)", inner.expr),
                    },
                }),
                IrType::String => emit_string_cast(expr, inner, indent, state, string_storage),
                _ => Ok(inner),
            }
        }
        IrExprKind::Call { func, args } => {
            let mut prelude = String::new();
            let mut rendered_args = Vec::with_capacity(args.len());
            for arg in args {
                let rendered = emit_expr(arg, indent, state, BufferStorage::Automatic)?;
                prelude.push_str(&rendered.prelude);
                rendered_args.push(rendered.expr);
            }
            Ok(GeneratedExpr {
                prelude,
                expr: format!("{}({})", func.join("."), rendered_args.join(", ")),
            })
        }
        IrExprKind::Array(elems) => {
            let mut prelude = String::new();
            let mut rendered_elems = Vec::with_capacity(elems.len());
            for elem in elems {
                let rendered = emit_expr(elem, indent, state, BufferStorage::Automatic)?;
                prelude.push_str(&rendered.prelude);
                rendered_elems.push(rendered.expr);
            }
            Ok(GeneratedExpr {
                prelude,
                expr: format!("{{{}}}", rendered_elems.join(", ")),
            })
        }
        IrExprKind::Index { object, index } => {
            let object = emit_expr(object, indent, state, BufferStorage::Automatic)?;
            let index = emit_expr(index, indent, state, BufferStorage::Automatic)?;
            Ok(GeneratedExpr {
                prelude: format!("{}{}", object.prelude, index.prelude),
                expr: format!("{}[{}]", object.expr, index.expr),
            })
        }
        IrExprKind::Field { object, field } => {
            let object = emit_expr(object, indent, state, BufferStorage::Automatic)?;
            Ok(GeneratedExpr {
                prelude: object.prelude,
                expr: format!("{}.{}", object.expr, field),
            })
        }
    }
}

#[cfg(test)]
fn generate_expr(expr: &IrExpr) -> String {
    let mut state = BodyCodegenState::default();
    let rendered = emit_expr(expr, "", &mut state, BufferStorage::Automatic)
        .expect("expression codegen should succeed in test helper");
    assert!(
        rendered.prelude.is_empty(),
        "expression requires statement-local prelude for safe codegen"
    );
    rendered.expr
}

fn c_type_name(ty: &IrType) -> &'static str {
    match ty {
        IrType::Bool => "bool",
        IrType::Float => "float",
        IrType::String => "const char*",
        IrType::Pin => "int",
        IrType::Array(_) => "int",
        IrType::Unit(UnitCategory::Time) => "int",
        IrType::Unit(_) => "float",
        IrType::Int | IrType::Sensor | IrType::Output | IrType::Unknown => "int",
        IrType::Void => "void",
    }
}

fn default_initializer(ty: &IrType) -> &'static str {
    match ty {
        IrType::Bool => "false",
        IrType::Float => "0.0f",
        IrType::Unit(UnitCategory::Time) => "0",
        IrType::Unit(_) => "0.0f",
        IrType::String => "\"\"",
        IrType::Void => "",
        _ => "0",
    }
}

fn format_params(params: &[(String, IrType)]) -> String {
    if params.is_empty() {
        "void".to_string()
    } else {
        params
            .iter()
            .map(|(name, ty)| format!("{} {}", c_type_name(ty), name))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn format_extern_signature(ext: &IrExtern) -> String {
    format!(
        "{} {}({})",
        c_type_name(&ext.return_type),
        ext.name.join("."),
        format_params(&ext.params)
    )
}

fn format_energy_policy(policy: &EnergyPolicy) -> String {
    format!(
        "default_sleep={}, wake_sources=[{}], overrun_policy={}",
        power_state_name(&policy.default_sleep_state),
        format_wake_sources(&policy.allowed_wake_sources),
        overrun_policy_name(&policy.overrun_policy)
    )
}

pub fn format_program_metadata(program: &IrProgram) -> String {
    let mut code = String::new();
    code.push_str(&format!(
        "/* Scheduler model: {} */\n",
        scheduler_model_name(&program.scheduler.model)
    ));
    code.push_str(&format!(
        "/* Energy policy: {} */\n\n",
        format_energy_policy(&program.energy_policy)
    ));

    let sensors = program
        .definitions
        .iter()
        .filter_map(|def| match def {
            IrDefinition::Sensor(sensor) => Some(sensor),
            _ => None,
        })
        .collect::<Vec<_>>();
    let outputs = program
        .definitions
        .iter()
        .filter_map(|def| match def {
            IrDefinition::Output(output) => Some(output),
            _ => None,
        })
        .collect::<Vec<_>>();
    if !sensors.is_empty() || !outputs.is_empty() {
        code.push_str("/* I/O definitions:\n");
        for sensor in sensors {
            if let (Some(category), Some(converter)) = (&sensor.category, &sensor.converter) {
                code.push_str(&format!(
                    " * sensor {} on {} : {} using {}\n",
                    sensor.name,
                    sensor.pin,
                    category,
                    converter.join(".")
                ));
            } else {
                code.push_str(&format!(" * sensor {} on {}\n", sensor.name, sensor.pin));
            }
        }
        for output in outputs {
            code.push_str(&format!(" * output {} on {}\n", output.name, output.pin));
        }
        code.push_str(" */\n\n");
    }

    let units = program
        .definitions
        .iter()
        .filter_map(|def| match def {
            IrDefinition::Unit(unit) => Some(unit),
            _ => None,
        })
        .collect::<Vec<_>>();
    if !units.is_empty() {
        code.push_str("/* Custom units:\n");
        for unit in units {
            code.push_str(&format!(
                " * {} [{}] to_base={} from_base={}\n",
                unit.name, unit.category, unit.to_base, unit.from_base
            ));
        }
        code.push_str(" */\n\n");
    }

    let externs = program
        .definitions
        .iter()
        .filter_map(|def| match def {
            IrDefinition::Extern(ext) => Some(ext),
            _ => None,
        })
        .collect::<Vec<_>>();
    if !externs.is_empty() {
        code.push_str("/* External dependencies:\n");
        for ext in &externs {
            code.push_str(&format!(" * {}\n", format_extern_signature(ext)));
        }
        code.push_str(" */\n");
        code.push('\n');
    }

    code
}

fn collect_local_decls(
    stmts: &[IrStmt],
    depth: usize,
    locals: &mut HashMap<String, IrType>,
    first_depths: &mut HashMap<String, usize>,
    predeclared: &HashSet<String>,
) {
    for stmt in stmts {
        match stmt {
            IrStmt::Read {
                variable,
                value_type,
                ..
            } => {
                if !predeclared.contains(variable) {
                    locals.entry(variable.clone()).or_insert(value_type.clone());
                    first_depths.entry(variable.clone()).or_insert(depth);
                }
            }
            IrStmt::Assign {
                variable, value, ..
            } => {
                if !predeclared.contains(variable) {
                    locals.entry(variable.clone()).or_insert(value.ty.clone());
                    first_depths.entry(variable.clone()).or_insert(depth);
                }
            }
            IrStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_local_decls(then_body, depth + 1, locals, first_depths, predeclared);
                if let Some(body) = else_body {
                    collect_local_decls(body, depth + 1, locals, first_depths, predeclared);
                }
            }
            IrStmt::While { body, .. } => {
                collect_local_decls(body, depth + 1, locals, first_depths, predeclared);
            }
            IrStmt::PeriodicBlock { body, .. } => {
                collect_local_decls(body, depth, locals, first_depths, predeclared);
            }
            IrStmt::For { variable, body, .. } => {
                let mut nested_predeclared = predeclared.clone();
                nested_predeclared.insert(variable.clone());
                collect_local_decls(body, depth + 1, locals, first_depths, &nested_predeclared);
            }
            IrStmt::Write { .. }
            | IrStmt::Sleep { .. }
            | IrStmt::Return { .. }
            | IrStmt::Expr(_) => {}
        }
    }
}

fn generate_stmt(
    stmt: &IrStmt,
    indent: &str,
    inline_decls: &mut HashSet<String>,
    state: &mut BodyCodegenState,
) -> CodegenResult<String> {
    let mut code = String::new();
    match stmt {
        IrStmt::Read {
            sensor,
            variable,
            value_type,
            converter,
            ..
            } => {
            let raw_value = format!("analogRead({})", sensor);
            let value = if let Some(converter) = converter {
                format!("{}({})", converter.join("."), raw_value)
            } else {
                raw_value
            };
            if inline_decls.remove(variable) {
                code.push_str(&format!(
                    "{}{} {} = {};\n",
                    indent,
                    c_type_name(value_type),
                    variable,
                    value
                ));
            } else {
                code.push_str(&format!("{}{} = {};\n", indent, variable, value));
            }
        }
        IrStmt::Write { output, value, .. } => {
            let rendered = emit_expr(value, indent, state, BufferStorage::Automatic)?;
            code.push_str(&rendered.prelude);
            code.push_str(&format!(
                "{}analogWrite({}, {});\n",
                indent,
                output,
                rendered.expr
            ));
        }
        IrStmt::Expr(expr) => {
            let rendered = emit_expr(expr, indent, state, BufferStorage::Automatic)?;
            code.push_str(&rendered.prelude);
            code.push_str(&format!("{}{};\n", indent, rendered.expr));
        }
        IrStmt::Assign {
            variable, value, ..
        } => {
            let storage = if value.ty == IrType::String {
                BufferStorage::Static
            } else {
                BufferStorage::Automatic
            };
            let rendered = emit_expr(value, indent, state, storage)?;
            code.push_str(&rendered.prelude);
            if inline_decls.remove(variable) {
                code.push_str(&format!(
                    "{}{} {} = {};\n",
                    indent,
                    c_type_name(&value.ty),
                    variable,
                    rendered.expr
                ));
            } else {
                code.push_str(&format!("{}{} = {};\n", indent, variable, rendered.expr));
            }
        }
        IrStmt::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            let rendered = emit_expr(condition, indent, state, BufferStorage::Automatic)?;
            code.push_str(&rendered.prelude);
            code.push_str(&format!("{}if ({}) {{\n", indent, rendered.expr));
            let next_indent = format!("{}    ", indent);
            for s in then_body {
                code.push_str(&generate_stmt(s, &next_indent, inline_decls, state)?);
            }
            if let Some(eb) = else_body {
                code.push_str(&format!("{}}} else {{\n", indent));
                for s in eb {
                    code.push_str(&generate_stmt(s, &next_indent, inline_decls, state)?);
                }
            }
            code.push_str(&format!("{}}}\n", indent));
        }
        IrStmt::While {
            condition, body, ..
        } => {
            let rendered = emit_expr(condition, indent, state, BufferStorage::Automatic)?;
            code.push_str(&rendered.prelude);
            code.push_str(&format!(
                "{}while ({}) {{\n",
                indent,
                rendered.expr
            ));
            let next_indent = format!("{}    ", indent);
            for s in body {
                code.push_str(&generate_stmt(s, &next_indent, inline_decls, state)?);
            }
            code.push_str(&format!("{}}}\n", indent));
        }
        IrStmt::For {
            variable,
            iterable,
            body,
            ..
        } => {
            let loop_id = state.loop_counter;
            state.loop_counter += 1;

            let (elements, elem_type) = match (&iterable.kind, &iterable.ty) {
                (IrExprKind::Array(elements), IrType::Array(elem_type)) => {
                    (elements, elem_type.as_ref())
                }
                _ => {
                    return Err(CodegenError::unsupported_ir(
                        "for-loop iterable was not lowered to an array",
                        iterable.source_span,
                    ));
                }
            };

            let iter_values = format!("__lpc_iter_values_{}", loop_id);
            let iter_len = format!("__lpc_iter_len_{}", loop_id);
            let iter_idx = format!("__lpc_iter_idx_{}", loop_id);
            let mut rendered_elements = Vec::with_capacity(elements.len());
            for element in elements {
                let rendered = emit_expr(element, indent, state, BufferStorage::Automatic)?;
                code.push_str(&rendered.prelude);
                rendered_elements.push(rendered.expr);
            }
            let elements_str = if rendered_elements.is_empty() {
                default_initializer(elem_type).to_string()
            } else {
                rendered_elements.join(", ")
            };
            let iter_len_value = elements.len();

            code.push_str(&format!(
                "{}{} {}[] = {{{}}};\n",
                indent,
                c_type_name(elem_type),
                iter_values,
                elements_str
            ));
            code.push_str(&format!(
                "{}int {} = {};\n",
                indent, iter_len, iter_len_value
            ));
            code.push_str(&format!(
                "{}for (int {} = 0; {} < {}; ++{}) {{\n",
                indent, iter_idx, iter_idx, iter_len, iter_idx
            ));
            code.push_str(&format!(
                "{}    {} {} = {}[{}];\n",
                indent,
                c_type_name(elem_type),
                variable,
                iter_values,
                iter_idx
            ));
            let next_indent = format!("{}    ", indent);
            for s in body {
                code.push_str(&generate_stmt(s, &next_indent, inline_decls, state)?);
            }
            code.push_str(&format!("{}}}\n", indent));
        }
        IrStmt::Return { value, .. } => {
            if let Some(v) = value {
                let storage = if v.ty == IrType::String {
                    BufferStorage::Static
                } else {
                    BufferStorage::Automatic
                };
                let rendered = emit_expr(v, indent, state, storage)?;
                code.push_str(&rendered.prelude);
                code.push_str(&format!("{}return {};\n", indent, rendered.expr));
            } else {
                code.push_str(&format!("{}return;\n", indent));
            }
        }
        IrStmt::PeriodicBlock { body, .. } => {
            for b in body {
                code.push_str(&generate_stmt(b, indent, inline_decls, state)?);
            }
        }
        IrStmt::Sleep {
            duration_micros,
            mode_hint,
            wake_sources,
            fallback,
            ..
        } => {
            if let Some(mode_hint) = mode_hint {
                code.push_str(&format!(
                    "{}/* sleep mode={} wake_sources=[{}] fallback={} */\n",
                    indent,
                    power_state_name(mode_hint),
                    format_wake_sources(wake_sources),
                    sleep_fallback_name(fallback)
                ));
            }

            match duration_micros {
                Some(duration_micros) if *duration_micros > 0 => {
                    code.push_str(&format!(
                        "{}__lpc_low_power_sleep_micros({}ULL);\n",
                        indent, duration_micros
                    ));
                }
                _ => {
                    return Err(CodegenError::unsupported_ir(
                        "sleep duration could not be lowered to a positive microsecond value",
                        stmt_source_span(stmt),
                    ));
                }
            }
        }
    }
    Ok(code)
}

fn generate_scoped_body(
    stmts: &[IrStmt],
    indent: &str,
    predeclared: &HashSet<String>,
) -> CodegenResult<String> {
    let mut locals = HashMap::new();
    let mut first_depths = HashMap::new();
    collect_local_decls(stmts, 0, &mut locals, &mut first_depths, predeclared);

    let mut code = String::new();
    let mut local_names = locals.keys().cloned().collect::<Vec<_>>();
    local_names.sort();
    for name in local_names {
        let ty = locals
            .get(&name)
            .expect("local name collected from locals map should exist");
        if first_depths.get(&name).copied().unwrap_or(0) > 0 {
            code.push_str(&format!(
                "{}{} {} = {};\n",
                indent,
                c_type_name(ty),
                name,
                default_initializer(ty)
            ));
        }
    }
    if !locals.is_empty() {
        code.push('\n');
    }

    let mut inline_decls = first_depths
        .into_iter()
        .filter_map(|(name, depth)| if depth == 0 { Some(name) } else { None })
        .collect::<HashSet<_>>();
    let mut state = BodyCodegenState::default();
    for stmt in stmts {
        code.push_str(&generate_stmt(stmt, indent, &mut inline_decls, &mut state)?);
    }
    Ok(code)
}

pub fn task_symbol(task: &IrTask, idx: usize) -> String {
    match &task.name {
        Some(name) => format!("task_{}", name),
        None => format!("task_{}", idx),
    }
}

pub fn generate_task(idx: usize, task: &IrTask) -> CodegenResult<String> {
    let mut code = String::new();
    code.push_str(&source_span_comment(task.source_span, ""));
    code.push_str(&format!("void {}(void) {{\n", task_symbol(task, idx)));
    code.push_str(&generate_scoped_body(&task.body, "    ", &HashSet::new())?);
    code.push_str("}\n\n");
    Ok(code)
}

pub fn generate_setup_body(stmts: &[IrStmt], indent: &str) -> CodegenResult<String> {
    generate_scoped_body(stmts, indent, &HashSet::new())
}

pub fn generate_function(func: &IrFunction) -> CodegenResult<String> {
    let mut code = String::new();
    code.push_str(&source_span_comment(func.source_span, ""));
    code.push_str(&format!(
        "{} {}({}) {{\n",
        c_type_name(&func.return_type),
        func.name,
        format_params(&func.params)
    ));
    let predeclared = func
        .params
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<HashSet<_>>();
    code.push_str(&generate_scoped_body(&func.body, "    ", &predeclared)?);
    code.push_str("}\n\n");
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::{
        emit_expr, escape_c_string_literal, format_autocast_helpers, format_prelude,
        format_program_metadata, generate_expr, generate_function, generate_setup_body,
        generate_task, BodyCodegenState, BufferStorage,
    };
    use crate::ast::Number;
    use crate::config::{CompilerConfig, TargetArch};
    use crate::task_ir::{
        EnergyPolicy, IrBinOp, IrDefinition, IrExpr, IrExprKind, IrExtern, IrFunction, IrProgram,
        IrStmt, IrTask, IrType, SchedulerConfig, TaskTrigger,
    };

    #[test]
    fn test_generate_namespaced_call_uses_dots() {
        let expr = IrExpr {
            kind: IrExprKind::Call {
                func: vec!["Serial".to_string(), "println".to_string()],
                args: vec![IrExpr {
                    kind: IrExprKind::StringLit("hello".to_string()),
                    ty: IrType::String,
                    unit: None,
                    source_span: None,
                }],
            },
            ty: IrType::Void,
            unit: None,
            source_span: None,
        };

        assert_eq!(generate_expr(&expr), "Serial.println(\"hello\")");
    }

    #[test]
    fn test_generate_cast_to_string_uses_helper() {
        let expr = IrExpr {
            kind: IrExprKind::Cast {
                expr: Box::new(IrExpr {
                    kind: IrExprKind::IntLit(42),
                    ty: IrType::Int,
                    unit: None,
                    source_span: None,
                }),
                target: IrType::String,
            },
            ty: IrType::String,
            unit: None,
            source_span: None,
        };

        let mut state = BodyCodegenState::default();
        let rendered = emit_expr(&expr, "    ", &mut state, BufferStorage::Static)
            .expect("string cast should render");
        assert!(rendered.prelude.contains("static char __lpc_string_scalar_0[32];"));
        assert!(rendered
            .prelude
            .contains("__lpc_format_int_to_buffer(__lpc_string_scalar_0, sizeof(__lpc_string_scalar_0), 42);"));
        assert_eq!(rendered.expr, "__lpc_string_scalar_0");
    }

    #[test]
    fn test_generate_unit_cast_to_int_uses_c_cast() {
        let expr = IrExpr {
            kind: IrExprKind::Cast {
                expr: Box::new(IrExpr {
                    kind: IrExprKind::UnitLit {
                        value: crate::ast::Number::Int(42000),
                        unit: "us".to_string(),
                        category: crate::types::UnitCategory::Time,
                    },
                    ty: IrType::Unit(crate::types::UnitCategory::Time),
                    unit: Some(crate::types::UnitCategory::Time),
                    source_span: None,
                }),
                target: IrType::Int,
            },
            ty: IrType::Int,
            unit: None,
            source_span: None,
        };

        assert_eq!(generate_expr(&expr), "((int)(42000))");
    }

    #[test]
    fn test_escape_c_string_literal_quote_and_backslash() {
        assert_eq!(escape_c_string_literal("a\"b"), "a\\\"b");
        assert_eq!(escape_c_string_literal("a\\b"), "a\\\\b");
    }

    #[test]
    fn test_escape_c_string_literal_control_chars() {
        assert_eq!(escape_c_string_literal("a\nb\tc\rd"), "a\\nb\\tc\\rd");
        assert_eq!(escape_c_string_literal("a\0b"), "a\\0b");
    }

    #[test]
    fn test_generate_expr_preserves_integral_float_literal_as_float() {
        let expr = IrExpr {
            kind: IrExprKind::FloatLit(1023.0),
            ty: IrType::Float,
            unit: None,
            source_span: None,
        };

        assert_eq!(generate_expr(&expr), "1023.0f");
    }

    #[test]
    fn test_generate_expr_preserves_fractional_float_literal_suffix() {
        let expr = IrExpr {
            kind: IrExprKind::FloatLit(273.15),
            ty: IrType::Float,
            unit: None,
            source_span: None,
        };

        assert_eq!(generate_expr(&expr), "273.15f");
    }

    #[test]
    fn test_generate_expr_escapes_string_literal() {
        let expr = IrExpr {
            kind: IrExprKind::StringLit("a\"b\\c\n\td\re".to_string()),
            ty: IrType::String,
            unit: None,
            source_span: None,
        };

        assert_eq!(generate_expr(&expr), "\"a\\\"b\\\\c\\n\\td\\re\"");
    }

    #[test]
    fn test_generate_expr_sanitizes_non_finite_float_literals() {
        let nan_expr = IrExpr {
            kind: IrExprKind::FloatLit(f64::NAN),
            ty: IrType::Float,
            unit: None,
            source_span: None,
        };
        let pos_inf_expr = IrExpr {
            kind: IrExprKind::FloatLit(f64::INFINITY),
            ty: IrType::Float,
            unit: None,
            source_span: None,
        };
        let neg_inf_expr = IrExpr {
            kind: IrExprKind::FloatLit(f64::NEG_INFINITY),
            ty: IrType::Float,
            unit: None,
            source_span: None,
        };

        assert_eq!(generate_expr(&nan_expr), "0.0f");
        assert_eq!(generate_expr(&pos_inf_expr), "3.4028235e+38f");
        assert_eq!(generate_expr(&neg_inf_expr), "-3.4028235e+38f");
    }

    #[test]
    fn test_generate_expr_sanitizes_non_finite_unit_float_literals() {
        let expr = IrExpr {
            kind: IrExprKind::UnitLit {
                value: Number::Float(f64::INFINITY),
                unit: "c".to_string(),
                category: crate::types::UnitCategory::Temperature,
            },
            ty: IrType::Unit(crate::types::UnitCategory::Temperature),
            unit: Some(crate::types::UnitCategory::Temperature),
            source_span: None,
        };

        assert_eq!(generate_expr(&expr), "3.4028235e+38f");
    }

    #[test]
    fn test_format_autocast_helpers_contains_string_converter() {
        let helpers = format_autocast_helpers();
        assert!(helpers.contains("__lpc_format_int_to_buffer"));
        assert!(helpers.contains("__lpc_format_float_to_buffer"));
        assert!(helpers.contains("__lpc_format_bool_to_buffer"));
        assert!(helpers.contains("__lpc_to_bool_from_string"));
        assert!(helpers.contains("__lpc_append_cstr"));
        assert!(!helpers.contains("__lpc_autocast_string_slots"));
    }

    #[test]
    fn test_generate_string_concat_uses_helper() {
        let expr = IrExpr {
            kind: IrExprKind::BinaryOp {
                lhs: Box::new(IrExpr {
                    kind: IrExprKind::StringLit("A".to_string()),
                    ty: IrType::String,
                    unit: None,
                    source_span: None,
                }),
                op: IrBinOp::Add,
                rhs: Box::new(IrExpr {
                    kind: IrExprKind::Cast {
                        expr: Box::new(IrExpr {
                            kind: IrExprKind::IntLit(42),
                            ty: IrType::Int,
                            unit: None,
                            source_span: None,
                        }),
                        target: IrType::String,
                    },
                    ty: IrType::String,
                    unit: None,
                    source_span: None,
                }),
            },
            ty: IrType::String,
            unit: None,
            source_span: None,
        };

        let mut state = BodyCodegenState::default();
        let rendered = emit_expr(&expr, "    ", &mut state, BufferStorage::Automatic)
            .expect("string concat should render");
        assert!(rendered.prelude.contains("char __lpc_string_concat_0[128];"));
        assert!(rendered
            .prelude
            .contains("__lpc_append_cstr(__lpc_string_concat_0, sizeof(__lpc_string_concat_0), \"A\");"));
        assert!(rendered
            .prelude
            .contains("__lpc_format_int_to_buffer(__lpc_string_scalar_1, sizeof(__lpc_string_scalar_1), 42);"));
        assert!(rendered
            .prelude
            .contains("__lpc_append_cstr(__lpc_string_concat_0, sizeof(__lpc_string_concat_0), __lpc_string_scalar_1);"));
        assert_eq!(rendered.expr, "__lpc_string_concat_0");
    }

    #[test]
    fn test_generate_string_concat_escapes_literals() {
        let expr = IrExpr {
            kind: IrExprKind::BinaryOp {
                lhs: Box::new(IrExpr {
                    kind: IrExprKind::StringLit("L\n".to_string()),
                    ty: IrType::String,
                    unit: None,
                    source_span: None,
                }),
                op: IrBinOp::Add,
                rhs: Box::new(IrExpr {
                    kind: IrExprKind::StringLit("R\"\\".to_string()),
                    ty: IrType::String,
                    unit: None,
                    source_span: None,
                }),
            },
            ty: IrType::String,
            unit: None,
            source_span: None,
        };

        let mut state = BodyCodegenState::default();
        let rendered = emit_expr(&expr, "    ", &mut state, BufferStorage::Automatic)
            .expect("string concat should render");
        assert!(rendered
            .prelude
            .contains("__lpc_append_cstr(__lpc_string_concat_0, sizeof(__lpc_string_concat_0), \"L\\n\");"));
        assert!(rendered
            .prelude
            .contains("__lpc_append_cstr(__lpc_string_concat_0, sizeof(__lpc_string_concat_0), \"R\\\"\\\\\");"));
    }

    #[test]
    fn test_generate_namespaced_call_escapes_string_args() {
        let expr = IrExpr {
            kind: IrExprKind::Call {
                func: vec!["Serial".to_string(), "println".to_string()],
                args: vec![IrExpr {
                    kind: IrExprKind::StringLit("x\"y\\z\n".to_string()),
                    ty: IrType::String,
                    unit: None,
                    source_span: None,
                }],
            },
            ty: IrType::Void,
            unit: None,
            source_span: None,
        };

        assert_eq!(generate_expr(&expr), "Serial.println(\"x\\\"y\\\\z\\n\")");
    }

    #[test]
    fn test_generate_call_with_string_concat_uses_statement_local_prelude() {
        let concat_arg = IrExpr {
            kind: IrExprKind::BinaryOp {
                lhs: Box::new(IrExpr {
                    kind: IrExprKind::StringLit("A=".to_string()),
                    ty: IrType::String,
                    unit: None,
                    source_span: None,
                }),
                op: IrBinOp::Add,
                rhs: Box::new(IrExpr {
                    kind: IrExprKind::Cast {
                        expr: Box::new(IrExpr {
                            kind: IrExprKind::IntLit(7),
                            ty: IrType::Int,
                            unit: None,
                            source_span: None,
                        }),
                        target: IrType::String,
                    },
                    ty: IrType::String,
                    unit: None,
                    source_span: None,
                }),
            },
            ty: IrType::String,
            unit: None,
            source_span: None,
        };
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::Expr(IrExpr {
                kind: IrExprKind::Call {
                    func: vec!["Serial".to_string(), "println".to_string()],
                    args: vec![concat_arg],
                },
                ty: IrType::Void,
                unit: None,
                source_span: None,
            })],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        let prelude_pos = code
            .find("char __lpc_string_concat_0[128];")
            .expect("expected statement-local concat prelude");
        let call_pos = code
            .find("Serial.println(__lpc_string_concat_0);")
            .expect("expected concat temp to be passed to call");
        assert!(prelude_pos < call_pos, "expected prelude before call:\n{}", code);
    }

    #[test]
    fn test_generate_string_return_uses_static_buffer_prelude() {
        let func = IrFunction {
            name: "render".to_string(),
            params: vec![],
            return_type: IrType::String,
            body: vec![IrStmt::Return {
                value: Some(IrExpr {
                    kind: IrExprKind::BinaryOp {
                        lhs: Box::new(IrExpr {
                            kind: IrExprKind::StringLit("V=".to_string()),
                            ty: IrType::String,
                            unit: None,
                            source_span: None,
                        }),
                        op: IrBinOp::Add,
                        rhs: Box::new(IrExpr {
                            kind: IrExprKind::Cast {
                                expr: Box::new(IrExpr {
                                    kind: IrExprKind::IntLit(3),
                                    ty: IrType::Int,
                                    unit: None,
                                    source_span: None,
                                }),
                                target: IrType::String,
                            },
                            ty: IrType::String,
                            unit: None,
                            source_span: None,
                        }),
                    },
                    ty: IrType::String,
                    unit: None,
                    source_span: None,
                }),
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_function(&func).expect("function codegen should succeed");
        assert!(code.contains("static char __lpc_string_concat_0[128];"));
        assert!(code.contains("return __lpc_string_concat_0;"));
    }

    #[test]
    fn test_generate_string_assignment_uses_static_buffer_prelude() {
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::Assign {
                variable: "msg".to_string(),
                value: IrExpr {
                    kind: IrExprKind::BinaryOp {
                        lhs: Box::new(IrExpr {
                            kind: IrExprKind::StringLit("T=".to_string()),
                            ty: IrType::String,
                            unit: None,
                            source_span: None,
                        }),
                        op: IrBinOp::Add,
                        rhs: Box::new(IrExpr {
                            kind: IrExprKind::Cast {
                                expr: Box::new(IrExpr {
                                    kind: IrExprKind::IntLit(9),
                                    ty: IrType::Int,
                                    unit: None,
                                    source_span: None,
                                }),
                                target: IrType::String,
                            },
                            ty: IrType::String,
                            unit: None,
                            source_span: None,
                        }),
                    },
                    ty: IrType::String,
                    unit: None,
                    source_span: None,
                },
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        assert!(code.contains("static char __lpc_string_concat_0[128];"));
        assert!(code.contains("const char* msg = __lpc_string_concat_0;"));
    }

    #[test]
    fn test_generate_task_declares_top_level_assignment() {
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::Assign {
                variable: "t".to_string(),
                value: IrExpr {
                    kind: IrExprKind::IntLit(0),
                    ty: IrType::Int,
                    unit: None,
                    source_span: None,
                },
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        assert!(code.contains("int t = 0;"));
    }

    #[test]
    fn test_generate_write_statement_emits_analog_write() {
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::Write {
                output: "buzz".to_string(),
                value: IrExpr {
                    kind: IrExprKind::IntLit(255),
                    ty: IrType::Int,
                    unit: None,
                    source_span: None,
                },
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        assert!(code.contains("analogWrite(buzz, 255);"));
    }

    #[test]
    fn test_generate_task_predeclares_nested_assignment() {
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![
                IrStmt::If {
                    condition: IrExpr {
                        kind: IrExprKind::BoolLit(true),
                        ty: IrType::Bool,
                        unit: None,
                        source_span: None,
                    },
                    then_body: vec![IrStmt::Assign {
                        variable: "t".to_string(),
                        value: IrExpr {
                            kind: IrExprKind::IntLit(1),
                            ty: IrType::Int,
                            unit: None,
                            source_span: None,
                        },
                        source_span: None,
                    }],
                    else_body: None,
                    source_span: None,
                },
                IrStmt::Assign {
                    variable: "t".to_string(),
                    value: IrExpr {
                        kind: IrExprKind::IntLit(2),
                        ty: IrType::Int,
                        unit: None,
                        source_span: None,
                    },
                    source_span: None,
                },
            ],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        assert!(code.contains("int t = 0;"));
        assert!(code.contains("t = 1;"));
        assert!(code.contains("t = 2;"));
    }

    #[test]
    fn test_generate_task_predeclares_nested_locals_in_deterministic_order() {
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::If {
                condition: IrExpr {
                    kind: IrExprKind::BoolLit(true),
                    ty: IrType::Bool,
                    unit: None,
                    source_span: None,
                },
                then_body: vec![
                    IrStmt::Assign {
                        variable: "zeta".to_string(),
                        value: IrExpr {
                            kind: IrExprKind::IntLit(1),
                            ty: IrType::Int,
                            unit: None,
                            source_span: None,
                        },
                        source_span: None,
                    },
                    IrStmt::Assign {
                        variable: "alpha".to_string(),
                        value: IrExpr {
                            kind: IrExprKind::IntLit(2),
                            ty: IrType::Int,
                            unit: None,
                            source_span: None,
                        },
                        source_span: None,
                    },
                ],
                else_body: None,
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        let alpha_pos = code
            .find("int alpha = 0;")
            .expect("expected alpha predeclaration");
        let zeta_pos = code
            .find("int zeta = 0;")
            .expect("expected zeta predeclaration");
        assert!(
            alpha_pos < zeta_pos,
            "expected deterministic alphabetical predeclaration order, got:\n{}",
            code
        );
    }

    #[test]
    fn test_generate_for_loop_uses_c_indexed_temp_array() {
        let iterable = IrExpr {
            kind: IrExprKind::Array(vec![
                IrExpr {
                    kind: IrExprKind::IntLit(1),
                    ty: IrType::Int,
                    unit: None,
                    source_span: None,
                },
                IrExpr {
                    kind: IrExprKind::IntLit(2),
                    ty: IrType::Int,
                    unit: None,
                    source_span: None,
                },
            ]),
            ty: IrType::Array(Box::new(IrType::Int)),
            unit: None,
            source_span: None,
        };
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::For {
                variable: "i".to_string(),
                iterable,
                body: vec![IrStmt::Assign {
                    variable: "x".to_string(),
                    value: IrExpr {
                        kind: IrExprKind::Ident("i".to_string()),
                        ty: IrType::Int,
                        unit: None,
                        source_span: None,
                    },
                    source_span: None,
                }],
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        assert!(code.contains("int __lpc_iter_len_0 = 2;"));
        assert!(code.contains("for (int __lpc_iter_idx_0 = 0; __lpc_iter_idx_0 < __lpc_iter_len_0; ++__lpc_iter_idx_0)"));
        assert!(code.contains("int i = __lpc_iter_values_0[__lpc_iter_idx_0];"));
    }

    #[test]
    fn test_generate_for_loop_empty_array_emits_zero_length() {
        let iterable = IrExpr {
            kind: IrExprKind::Array(vec![]),
            ty: IrType::Array(Box::new(IrType::Int)),
            unit: None,
            source_span: None,
        };
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::For {
                variable: "i".to_string(),
                iterable,
                body: vec![],
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        assert!(code.contains("int __lpc_iter_len_0 = 0;"));
    }

    #[test]
    fn test_generate_for_loop_rejects_non_array_iterable() {
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::For {
                variable: "i".to_string(),
                iterable: IrExpr {
                    kind: IrExprKind::Ident("xs".to_string()),
                    ty: IrType::Array(Box::new(IrType::Int)),
                    unit: None,
                    source_span: None,
                },
                body: vec![],
                source_span: None,
            }],
            source_span: None,
        };

        let err = generate_task(0, &task).expect_err("task codegen should fail");
        assert!(err.to_string().contains("for-loop iterable was not lowered to an array"));
    }

    #[test]
    fn test_generate_task_periodic_block_does_not_duplicate_top_level_initialization() {
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::PeriodicBlock {
                body: vec![
                    IrStmt::Assign {
                        variable: "temp".to_string(),
                        value: IrExpr {
                            kind: IrExprKind::FloatLit(0.0),
                            ty: IrType::Float,
                            unit: None,
                            source_span: None,
                        },
                        source_span: None,
                    },
                    IrStmt::Assign {
                        variable: "hum".to_string(),
                        value: IrExpr {
                            kind: IrExprKind::FloatLit(0.0),
                            ty: IrType::Float,
                            unit: None,
                            source_span: None,
                        },
                        source_span: None,
                    },
                ],
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        assert!(code.contains("float temp = 0.0f;"));
        assert!(code.contains("float hum = 0.0f;"));
        assert_eq!(code.matches("\n    temp = 0.0f;\n").count(), 0);
        assert_eq!(code.matches("\n    hum = 0.0f;\n").count(), 0);
    }

    #[test]
    fn test_generate_task_uses_float_for_non_time_units_and_int_for_time_units() {
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::PeriodicBlock {
                body: vec![
                    IrStmt::Assign {
                        variable: "temp".to_string(),
                        value: IrExpr {
                            kind: IrExprKind::UnitLit {
                                value: Number::Float(25.5),
                                unit: "c".to_string(),
                                category: crate::types::UnitCategory::Temperature,
                            },
                            ty: IrType::Unit(crate::types::UnitCategory::Temperature),
                            unit: Some(crate::types::UnitCategory::Temperature),
                            source_span: None,
                        },
                        source_span: None,
                    },
                    IrStmt::Assign {
                        variable: "pause_us".to_string(),
                        value: IrExpr {
                            kind: IrExprKind::UnitLit {
                                value: Number::Int(100),
                                unit: "us".to_string(),
                                category: crate::types::UnitCategory::Time,
                            },
                            ty: IrType::Unit(crate::types::UnitCategory::Time),
                            unit: Some(crate::types::UnitCategory::Time),
                            source_span: None,
                        },
                        source_span: None,
                    },
                ],
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        assert!(code.contains("float temp = 25.5f;"));
        assert!(code.contains("int pause_us = 100;"));
    }

    #[test]
    fn test_format_prelude() {
        let config = CompilerConfig {
            arch: TargetArch::Avr,
            clock_hz: 16_000_000,
            c_includes: vec![],
            c_prelude: vec![
                "#define DHTPIN 2".to_string(),
                "DHT dht(DHTPIN, DHTTYPE);".to_string(),
            ],
        };

        let prelude = format_prelude(&config);

        assert!(prelude.contains("#define DHTPIN 2"));
        assert!(prelude.contains("DHT dht(DHTPIN, DHTTYPE);"));
    }

    #[test]
    fn test_generate_function_uses_signature_and_body() {
        let func = IrFunction {
            name: "answer".to_string(),
            params: vec![("x".to_string(), IrType::Int)],
            return_type: IrType::Int,
            body: vec![IrStmt::Return {
                value: Some(IrExpr {
                    kind: IrExprKind::Ident("x".to_string()),
                    ty: IrType::Int,
                    unit: None,
                    source_span: None,
                }),
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_function(&func).expect("function codegen should succeed");
        assert!(code.contains("int answer(int x)"));
        assert!(code.contains("return x;"));
    }

    #[test]
    fn test_generate_setup_body_declares_top_level_assignment() {
        let stmts = vec![IrStmt::Assign {
            variable: "boot".to_string(),
            value: IrExpr {
                kind: IrExprKind::IntLit(1),
                ty: IrType::Int,
                unit: None,
                source_span: None,
            },
            source_span: None,
        }];

        let code = generate_setup_body(&stmts, "    ").expect("setup codegen should succeed");
        assert!(code.contains("int boot = 1;"));
    }

    #[test]
    fn test_generate_setup_body_uses_low_power_sleep_helper() {
        let stmts = vec![IrStmt::Sleep {
            duration_micros: Some(1_500),
            mode_hint: Some(crate::task_ir::PowerState::Idle),
            wake_sources: vec![crate::task_ir::WakeSource::Timer],
            fallback: crate::task_ir::SleepFallback::UseActiveDelay,
            source_span: None,
        }];

        let code = generate_setup_body(&stmts, "    ").expect("setup codegen should succeed");
        assert!(code.contains("__lpc_low_power_sleep_micros(1500ULL);"));
        assert!(!code.contains("delay(1);"));
        assert!(!code.contains("delayMicroseconds(500);"));
    }

    #[test]
    fn test_generate_setup_body_rejects_invalid_sleep_payload() {
        let stmts = vec![IrStmt::Sleep {
            duration_micros: None,
            mode_hint: Some(crate::task_ir::PowerState::Idle),
            wake_sources: vec![crate::task_ir::WakeSource::Timer],
            fallback: crate::task_ir::SleepFallback::UseActiveDelay,
            source_span: None,
        }];

        let err = generate_setup_body(&stmts, "    ").expect_err("setup codegen should fail");
        assert!(err
            .to_string()
            .contains("sleep duration could not be lowered to a positive microsecond value"));
    }

    #[test]
    fn test_format_program_metadata_uses_units_and_externs() {
        let program = IrProgram {
            definitions: vec![
                IrDefinition::Unit(crate::task_ir::IrUnitDef {
                    name: "fahrenheit".to_string(),
                    category: crate::types::UnitCategory::Temperature,
                    to_base: 1,
                    from_base: 1,
                }),
                IrDefinition::Extern(IrExtern {
                    name: vec!["printf".to_string()],
                    params: vec![("x".to_string(), IrType::String)],
                    return_type: IrType::Void,
                }),
            ],
            tasks: vec![],
            setup_body: vec![],
            functions: vec![],
            scheduler: SchedulerConfig::default(),
            energy_policy: EnergyPolicy::default(),
        };

        let metadata = format_program_metadata(&program);
        assert!(metadata.contains("Scheduler model: tick-driven"));
        assert!(metadata.contains("Energy policy: default_sleep=idle"));
        assert!(metadata.contains("fahrenheit [temperature]"));
        assert!(metadata.contains(" * void printf(const char* x)"));
        assert!(!metadata.contains("\nextern void printf(const char* x);\n"));
    }

    #[test]
    fn test_format_program_metadata_renders_custom_category_name() {
        let program = IrProgram {
            definitions: vec![IrDefinition::Unit(crate::task_ir::IrUnitDef {
                name: "psi".to_string(),
                category: crate::types::UnitCategory::Custom("pressure".to_string()),
                to_base: 6894,
                from_base: 1,
            })],
            tasks: vec![],
            setup_body: vec![],
            functions: vec![],
            scheduler: SchedulerConfig::default(),
            energy_policy: EnergyPolicy::default(),
        };

        let metadata = format_program_metadata(&program);
        assert!(metadata.contains("psi [pressure] to_base=6894 from_base=1"));
    }

    #[test]
    fn test_format_program_metadata_renders_io_definitions() {
        let program = IrProgram {
            definitions: vec![
                IrDefinition::Sensor(crate::task_ir::IrSensor {
                    name: "temp".to_string(),
                    pin: "A0".to_string(),
                    category: None,
                    converter: None,
                }),
                IrDefinition::Output(crate::task_ir::IrOutput {
                    name: "buzz".to_string(),
                    pin: "D0".to_string(),
                }),
            ],
            tasks: vec![],
            setup_body: vec![],
            functions: vec![],
            scheduler: SchedulerConfig::default(),
            energy_policy: EnergyPolicy::default(),
        };

        let metadata = format_program_metadata(&program);
        assert!(metadata.contains("sensor temp on A0"));
        assert!(metadata.contains("output buzz on D0"));
    }

    #[test]
    fn cg_typed_non_time_sensor_read_declares_float() {
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::PeriodicBlock {
                body: vec![IrStmt::Read {
                    sensor: "temp".to_string(),
                    variable: "t".to_string(),
                    value_type: IrType::Unit(crate::types::UnitCategory::Temperature),
                    converter: Some(vec!["Sensor".to_string(), "convert".to_string()]),
                    source_span: None,
                }],
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        assert!(code.contains("float t = Sensor.convert(analogRead(temp));"));
    }

    #[test]
    fn cg_typed_time_sensor_read_declares_int() {
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::PeriodicBlock {
                body: vec![IrStmt::Read {
                    sensor: "tick_sensor".to_string(),
                    variable: "t".to_string(),
                    value_type: IrType::Unit(crate::types::UnitCategory::Time),
                    converter: Some(vec!["Clock".to_string(), "convertTick".to_string()]),
                    source_span: None,
                }],
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        assert!(code.contains("int t = Clock.convertTick(analogRead(tick_sensor));"));
    }

    #[test]
    fn cg_converter_path_multi_segment_uses_dots() {
        let task = IrTask {
            name: None,
            trigger: TaskTrigger::Periodic {
                period_ticks: 1,
                phase_ticks: 0,
            },
            body: vec![IrStmt::PeriodicBlock {
                body: vec![IrStmt::Read {
                    sensor: "temp".to_string(),
                    variable: "t".to_string(),
                    value_type: IrType::Unit(crate::types::UnitCategory::Temperature),
                    converter: Some(vec![
                        "Vendor".to_string(),
                        "Sensors".to_string(),
                        "convert".to_string(),
                    ]),
                    source_span: None,
                }],
                source_span: None,
            }],
            source_span: None,
        };

        let code = generate_task(0, &task).expect("task codegen should succeed");
        assert!(code.contains("float t = Vendor.Sensors.convert(analogRead(temp));"));
    }
}
