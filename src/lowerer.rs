use crate::ast::{
    self, BinOp as AstBinOp, EveryBlock, Expr, ExprKind, FuncDef, Number, Program, Statement,
    TaskBlock, TopLevel, UnOp as AstUnOp,
};
use crate::task_ir::{
    self, IrBinOp, IrBlockKind, IrDefinition, IrError, IrExpr, IrExprKind, IrLoweringContext,
    IrProgram, IrResult, IrSensorReadInfo, IrStmt, IrTask, IrType, IrUnOp,
};
use crate::types::{self, UnitCategory};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
struct CustomTimeConversion {
    target_unit: String,
    to_target_expr: ast::ConversionExpr,
}

pub fn lower_program(program: &Program) -> IrResult<IrProgram> {
    let mut ctx = IrLoweringContext::new();
    let mut definitions = Vec::new();
    let mut tasks = Vec::new();
    let mut setup_body = Vec::new();
    let mut functions = Vec::new();
    let mut custom_time_conversions: HashMap<String, CustomTimeConversion> = HashMap::new();

    for top_level in &program.statements {
        match top_level {
            TopLevel::SensorDef(sensor) => {
                let (category, converter, read_type) = match (&sensor.category, &sensor.converter) {
                    (Some(category_name), Some(converter_path)) => {
                        let parsed =
                            types::parse_category(category_name).ok_or(IrError::TypeMismatch {
                                expected: IrType::Unit(UnitCategory::Time),
                                found: IrType::Unknown,
                                source_span: sensor.span,
                            })?;
                        (
                            Some(parsed.clone()),
                            Some(converter_path.clone()),
                            IrType::Unit(parsed),
                        )
                    }
                    (None, None) => (None, None, IrType::Int),
                    _ => {
                        return Err(IrError::TypeMismatch {
                            expected: IrType::Int,
                            found: IrType::Unknown,
                            source_span: sensor.span,
                        });
                    }
                };

                ctx.add_sensor(
                    &sensor.name,
                    &sensor.pin,
                    IrSensorReadInfo {
                        value_type: read_type,
                        converter: converter.clone(),
                    },
                );
                definitions.push(IrDefinition::Sensor(task_ir::IrSensor {
                    name: sensor.name.clone(),
                    pin: sensor.pin.clone(),
                    category,
                    converter,
                }));
            }
            TopLevel::OutputDef(output) => {
                ctx.add_output(&output.name, &output.pin);
                definitions.push(IrDefinition::Output(task_ir::IrOutput {
                    name: output.name.clone(),
                    pin: output.pin.clone(),
                }));
            }
            TopLevel::UnitDef(unit) => {
                let category =
                    types::parse_category(&unit.category).ok_or(IrError::TypeMismatch {
                        expected: IrType::Int,
                        found: IrType::Unit(UnitCategory::Time),
                        source_span: unit.span,
                    })?;
                register_custom_unit(&mut ctx, unit, category.clone())?;

                if category == UnitCategory::Time
                    && let Some(conversion) = extract_custom_time_conversion(unit)
                {
                    custom_time_conversions.insert(unit.name.clone(), conversion);
                }

                let (to_base, from_base) = calculate_unit_conversions(&unit.conversions)?;

                definitions.push(IrDefinition::Unit(task_ir::IrUnitDef {
                    name: unit.name.clone(),
                    category,
                    to_base,
                    from_base,
                }));
            }
            TopLevel::Extern(ext) => {
                let params = ext
                    .params
                    .iter()
                    .map(|(n, t)| (n.clone(), parse_ir_type(t.as_str())))
                    .collect();
                let ret = parse_ir_type(ext.ret.as_str());
                definitions.push(IrDefinition::Extern(task_ir::IrExtern {
                    name: ext.name.clone(),
                    params,
                    return_type: ret,
                }));
            }
            _ => {}
        }
    }

    for top_level in &program.statements {
        match top_level {
            TopLevel::Every(every) => {
                let task = lower_every_block(every, &mut ctx, &custom_time_conversions)?;
                tasks.push(task);
            }
            TopLevel::FuncDef(func) => {
                let ir_func = lower_function(func, &mut ctx, &custom_time_conversions)?;
                functions.push(ir_func);
            }
            TopLevel::Task(task) => {
                setup_body = lower_task_block(task, &mut ctx, &custom_time_conversions)?;
            }
            _ => {}
        }
    }

    Ok(IrProgram {
        definitions,
        tasks,
        setup_body,
        functions,
        scheduler: task_ir::SchedulerConfig::default(),
        energy_policy: task_ir::EnergyPolicy::default(),
    })
}
fn calculate_unit_conversions(
    conversions: &[(String, ast::ConversionExpr)],
) -> IrResult<(i64, i64)> {
    let mut to_base: Option<i64> = None;
    let mut from_base: Option<i64> = None;

    for (name, expr) in conversions {
        if name == "to_base" {
            to_base = Some(eval_conversion_expr(expr)?);
        } else if name == "from_base" {
            from_base = Some(eval_conversion_expr(expr)?);
        }
    }

    let to_base = to_base.unwrap_or(1);
    let from_base = from_base.unwrap_or(1);

    Ok((to_base, from_base))
}
fn eval_conversion_expr(expr: &ast::ConversionExpr) -> IrResult<i64> {
    match expr {
        ast::ConversionExpr::Lit(n) => Ok(*n as i64),
        ast::ConversionExpr::Val => Ok(1),
        ast::ConversionExpr::BinaryOp { lhs, op, rhs } => {
            let l = eval_conversion_expr(lhs)?;
            let r = eval_conversion_expr(rhs)?;
            match op {
                ast::BinOp::Add => Ok(l + r),
                ast::BinOp::Sub => Ok(l - r),
                ast::BinOp::Mul => Ok(l * r),
                ast::BinOp::Div => Ok(l / r),
                _ => Err(IrError::TypeMismatch {
                    expected: IrType::Int,
                    found: IrType::Float,
                    source_span: None,
                }),
            }
        }
        ast::ConversionExpr::Paren(e) => eval_conversion_expr(e),
        ast::ConversionExpr::UnaryNeg(e) => Ok(-eval_conversion_expr(e)?),
    }
}
fn lower_every_block(
    every: &EveryBlock,
    ctx: &mut IrLoweringContext,
    custom_time_conversions: &HashMap<String, CustomTimeConversion>,
) -> IrResult<IrTask> {
    let period_micros = time_value_to_micros(
        every.interval_value.clone(),
        &every.interval_unit,
        custom_time_conversions,
    )
    .ok_or(IrError::InvalidTaskPeriod {
        period_micros: 0,
        source_span: every.span,
    })?;
    if period_micros <= 0 {
        return Err(IrError::InvalidTaskPeriod {
            period_micros,
            source_span: every.span,
        });
    }

    let period_ticks = std::cmp::max(1, (period_micros as u32).div_ceil(1000));

    ctx.enter_block(IrBlockKind::Periodic);
    let mut inner_body = Vec::new();
    for stmt in &every.body {
        let ir_stmt = lower_statement(stmt, ctx, custom_time_conversions)?;
        inner_body.push(ir_stmt);
    }
    ctx.exit_block();

    let body = vec![IrStmt::PeriodicBlock {
        body: inner_body,
        source_span: every.span,
    }];

    Ok(IrTask {
        name: None,
        trigger: task_ir::TaskTrigger::Periodic {
            period_ticks,
            phase_ticks: 0,
        },
        body,
        source_span: every.span,
    })
}

fn lower_task_block(
    task: &TaskBlock,
    ctx: &mut IrLoweringContext,
    custom_time_conversions: &HashMap<String, CustomTimeConversion>,
) -> IrResult<Vec<IrStmt>> {
    ctx.enter_block(IrBlockKind::Setup);
    let mut body = Vec::new();
    for stmt in &task.body {
        let ir_stmt = lower_statement(stmt, ctx, custom_time_conversions)?;
        body.push(ir_stmt);
    }
    ctx.exit_block();
    Ok(body)
}

fn lower_function(
    func: &FuncDef,
    ctx: &mut IrLoweringContext,
    custom_time_conversions: &HashMap<String, CustomTimeConversion>,
) -> IrResult<task_ir::IrFunction> {
    for (name, _) in &func.params {
        ctx.add_variable(name);
    }
    let mut body = Vec::new();
    for stmt in &func.body {
        let ir_stmt = lower_statement(stmt, ctx, custom_time_conversions)?;
        body.push(ir_stmt);
    }

    Ok(task_ir::IrFunction {
        name: func.name.clone(),
        params: func
            .params
            .iter()
            .map(|(n, t)| (n.clone(), parse_ir_type(t.as_str())))
            .collect(),
        return_type: parse_ir_type(func.ret.as_str()),
        body,
        source_span: func.span,
    })
}
fn lower_statement(
    stmt: &Statement,
    ctx: &mut IrLoweringContext,
    custom_time_conversions: &HashMap<String, CustomTimeConversion>,
) -> IrResult<IrStmt> {
    match stmt {
        Statement::Read {
            sensor,
            variable,
            span,
        } => {
            if !ctx.has_sensor(sensor) {
                return Err(IrError::UnknownSensor {
                    name: sensor.clone(),
                    source_span: *span,
                });
            }
            let read_info = ctx
                .sensor_read_info(sensor)
                .cloned()
                .unwrap_or(IrSensorReadInfo {
                    value_type: IrType::Int,
                    converter: None,
                });
            let sensor_pin = ctx
                .sensor_pin(sensor)
                .cloned()
                .unwrap_or_else(|| sensor.clone());

            ctx.add_variable(variable);

            Ok(IrStmt::Read {
                sensor: sensor_pin,
                variable: variable.clone(),
                value_type: read_info.value_type,
                converter: read_info.converter,
                source_span: *span,
            })
        }
        Statement::Write {
            output,
            value,
            span,
        } => {
            if !ctx.has_output(output) {
                return Err(IrError::UnknownVariable {
                    name: output.clone(),
                    source_span: *span,
                });
            }
            let output_pin = ctx
                .output_pin(output)
                .cloned()
                .unwrap_or_else(|| output.clone());

            let ir_value = lower_expr(value, ctx)?;
            Ok(IrStmt::Write {
                output: output_pin,
                value: ir_value,
                source_span: *span,
            })
        }
        Statement::While {
            condition,
            body,
            span,
        } => {
            ctx.enter_block(IrBlockKind::Loop);
            let ir_condition = lower_expr(condition, ctx)?;
            let mut ir_body = Vec::new();
            for stmt in body {
                let ir_stmt = lower_statement(stmt, ctx, custom_time_conversions)?;
                if let IrStmt::Sleep { .. } = ir_stmt {
                    return Err(IrError::SleepInForbiddenContext { source_span: *span });
                }
                ir_body.push(ir_stmt);
            }
            ctx.exit_block();

            Ok(IrStmt::While {
                condition: ir_condition,
                body: ir_body,
                source_span: *span,
            })
        }
        Statement::For {
            variable,
            iterable,
            body,
            span,
        } => {
            ctx.add_variable(variable);
            ctx.enter_block(IrBlockKind::Loop);
            let ir_iterable = lower_expr(iterable, ctx)?;
            let mut ir_body = Vec::new();
            for stmt in body {
                let ir_stmt = lower_statement(stmt, ctx, custom_time_conversions)?;
                if let IrStmt::Sleep { .. } = ir_stmt {
                    return Err(IrError::SleepInForbiddenContext { source_span: *span });
                }
                ir_body.push(ir_stmt);
            }
            ctx.exit_block();

            Ok(IrStmt::For {
                variable: variable.clone(),
                iterable: ir_iterable,
                body: ir_body,
                source_span: *span,
            })
        }
        Statement::Sleep { value, unit, span } => {
            if !ctx.can_sleep_here() {
                if ctx.is_in_loop() {
                    return Err(IrError::SleepInForbiddenContext { source_span: *span });
                } else {
                    return Err(IrError::SleepOutsidePeriodicBlock { source_span: *span });
                }
            }
            let duration_micros = lower_sleep_duration(value, unit, custom_time_conversions)
                .ok_or(IrError::InvalidTaskPeriod {
                    period_micros: 0,
                    source_span: *span,
                })?;
            Ok(IrStmt::Sleep {
                duration_micros: Some(duration_micros),
                mode_hint: Some(task_ir::PowerState::Idle),
                wake_sources: vec![task_ir::WakeSource::Timer],
                fallback: task_ir::SleepFallback::UseActiveDelay,
                source_span: *span,
            })
        }
        Statement::Assignment {
            variable,
            value,
            span,
        } => {
            ctx.add_variable(variable);
            let ir_value = lower_expr(value, ctx)?;

            Ok(IrStmt::Assign {
                variable: variable.clone(),
                value: ir_value,
                source_span: *span,
            })
        }
        Statement::If {
            condition,
            then_body,
            else_body,
            span,
        } => {
            let ir_condition = lower_expr(condition, ctx)?;
            let mut ir_then = Vec::new();
            for stmt in then_body {
                let ir_stmt = lower_statement(stmt, ctx, custom_time_conversions)?;
                ir_then.push(ir_stmt);
            }
            let mut ir_else = None;
            if let Some(else_body) = else_body {
                let mut ir_else_body = Vec::new();
                for stmt in else_body {
                    let ir_stmt = lower_statement(stmt, ctx, custom_time_conversions)?;
                    ir_else_body.push(ir_stmt);
                }
                ir_else = Some(ir_else_body);
            }

            Ok(IrStmt::If {
                condition: ir_condition,
                then_body: ir_then,
                else_body: ir_else,
                source_span: *span,
            })
        }
        Statement::Return { value, span } => {
            let ir_value = match value {
                Some(expr) => Some(lower_expr(expr, ctx)?),
                None => None,
            };
            Ok(IrStmt::Return {
                value: ir_value,
                source_span: *span,
            })
        }
        Statement::Expr(expr) => {
            let ir_expr = lower_expr(expr, ctx)?;
            Ok(IrStmt::Expr(ir_expr))
        }
    }
}
fn lower_expr(expr: &Expr, ctx: &IrLoweringContext) -> IrResult<IrExpr> {
    let kind = lower_expr_kind(&expr.kind, ctx, &expr.unit, expr.span)?;
    let ty = expr
        .ty
        .as_ref()
        .map(IrType::from_ast)
        .unwrap_or(IrType::Int);
    let unit = expr.unit.clone();

    Ok(IrExpr {
        kind,
        ty,
        unit,
        source_span: expr.span,
    })
}
fn lower_expr_kind(
    kind: &ExprKind,
    ctx: &IrLoweringContext,
    expected_unit: &Option<UnitCategory>,
    source_span: Option<crate::diagnostics::SourceSpan>,
) -> IrResult<IrExprKind> {
    match kind {
        ExprKind::IntLit(i) => Ok(IrExprKind::IntLit(*i)),
        ExprKind::FloatLit(f) => Ok(IrExprKind::FloatLit(*f)),
        ExprKind::BoolLit(b) => Ok(IrExprKind::BoolLit(*b)),
        ExprKind::UnitLit { value, unit } => {
            let category = expected_unit
                .clone()
                .or_else(|| types::categorize_builtin_unit(unit))
                .or_else(|| ctx.unit_registry.categorize(unit))
                .ok_or(IrError::TypeMismatch {
                    expected: IrType::Unit(UnitCategory::Time),
                    found: IrType::Unknown,
                    source_span,
                })?;
            let normalized_value = if category == UnitCategory::Time {
                match value {
                    Number::Int(i) => Number::Int(*i),
                    Number::Float(f) => Number::Int(f.round() as i64),
                }
            } else {
                value.clone()
            };

            Ok(IrExprKind::UnitLit {
                value: normalized_value,
                unit: unit.clone(),
                category,
            })
        }
        ExprKind::Ident(name) => {
            if !ctx.has_variable(name) && !ctx.has_sensor(name) && !ctx.has_output(name) {
                return Err(IrError::UnknownVariable {
                    name: name.clone(),
                    source_span,
                });
            }
            Ok(IrExprKind::Ident(name.clone()))
        }
        ExprKind::BinaryOp { lhs, op, rhs } => {
            let ir_lhs = Box::new(lower_expr(lhs, ctx)?);
            let ir_rhs = Box::new(lower_expr(rhs, ctx)?);
            if let (Some(cat1), Some(cat2)) = (&lhs.unit, &rhs.unit) {
                if cat1 != cat2 {
                    return Err(IrError::TypeMismatch {
                        expected: IrType::Unit(cat1.clone()),
                        found: IrType::Unit(cat2.clone()),
                        source_span,
                    });
                }
                if matches!(
                    op,
                    AstBinOp::Mul | AstBinOp::Div | AstBinOp::Mod | AstBinOp::Pow
                ) {
                    return Err(IrError::TypeMismatch {
                        expected: IrType::Unknown,
                        found: IrType::Unit(cat2.clone()),
                        source_span,
                    });
                }
            }

            let ir_op = lower_bin_op(op);
            Ok(IrExprKind::BinaryOp {
                lhs: ir_lhs,
                op: ir_op,
                rhs: ir_rhs,
            })
        }
        ExprKind::UnaryOp { op, expr } => {
            let ir_expr = Box::new(lower_expr(expr, ctx)?);
            let ir_op = lower_un_op(op);
            Ok(IrExprKind::UnaryOp {
                op: ir_op,
                expr: ir_expr,
            })
        }
        ExprKind::Cast { expr, target } => {
            let ir_expr = Box::new(lower_expr(expr, ctx)?);
            Ok(IrExprKind::Cast {
                expr: ir_expr,
                target: IrType::from_ast(target),
            })
        }
        ExprKind::RangeArray { start, end } => {
            let mut ir_elements = Vec::new();
            for value in *start..*end {
                ir_elements.push(IrExpr {
                    kind: IrExprKind::IntLit(value),
                    ty: IrType::Int,
                    unit: None,
                    source_span,
                });
            }
            Ok(IrExprKind::Array(ir_elements))
        }
        ExprKind::Call { func, args } => {
            let func_expr = collect_call_path(func)?;

            let mut ir_args = Vec::new();
            for arg in args {
                ir_args.push(lower_expr(arg, ctx)?);
            }

            Ok(IrExprKind::Call {
                func: func_expr,
                args: ir_args,
            })
        }
        ExprKind::Paren(expr) => lower_expr_kind(&expr.kind, ctx, expected_unit, expr.span),
        ExprKind::Array(elements) => {
            let mut ir_elements = Vec::new();
            for element in elements {
                ir_elements.push(lower_expr(element, ctx)?);
            }
            Ok(IrExprKind::Array(ir_elements))
        }
        ExprKind::Index { object, index } => {
            let ir_object = Box::new(lower_expr(object, ctx)?);
            let ir_index = Box::new(lower_expr(index, ctx)?);
            Ok(IrExprKind::Index {
                object: ir_object,
                index: ir_index,
            })
        }
        ExprKind::Field { object, field } => {
            let ir_object = Box::new(lower_expr(object, ctx)?);
            Ok(IrExprKind::Field {
                object: ir_object,
                field: field.clone(),
            })
        }
        ExprKind::StringLit(s) => Ok(IrExprKind::StringLit(s.clone())),
    }
}

fn collect_call_path(expr: &Expr) -> IrResult<Vec<String>> {
    match &expr.kind {
        ExprKind::Ident(name) => Ok(vec![name.clone()]),
        ExprKind::Field { object, field } => {
            let mut path = collect_call_path(object)?;
            path.push(field.clone());
            Ok(path)
        }
        _ => Err(IrError::TypeMismatch {
            expected: IrType::Int,
            found: IrType::Void,
            source_span: expr.span,
        }),
    }
}

fn register_custom_unit(
    ctx: &mut IrLoweringContext,
    unit: &ast::UnitDef,
    category: UnitCategory,
) -> IrResult<()> {
    let to_base = unit
        .conversions
        .iter()
        .find(|(name, _)| name.starts_with("to_"))
        .map(|(_, expr)| expr.clone())
        .ok_or_else(|| IrError::TypeMismatch {
            expected: IrType::Unit(category.clone()),
            found: IrType::Unknown,
            source_span: unit.span,
        })?;
    let from_base = unit
        .conversions
        .iter()
        .find(|(name, _)| name.starts_with("from_"))
        .map(|(_, expr)| expr.clone())
        .ok_or_else(|| IrError::TypeMismatch {
            expected: IrType::Unit(category.clone()),
            found: IrType::Unknown,
            source_span: unit.span,
        })?;

    ctx.unit_registry
        .register(types::CustomUnitDef {
            name: unit.name.clone(),
            category,
            to_base,
            from_base,
        })
        .map_err(|_| IrError::TypeMismatch {
            expected: IrType::Int,
            found: IrType::Unknown,
            source_span: unit.span,
        })
}

fn extract_custom_time_conversion(unit: &ast::UnitDef) -> Option<CustomTimeConversion> {
    let (to_key, expr) = unit
        .conversions
        .iter()
        .find(|(name, _)| name.starts_with("to_"))?;
    let target_unit = to_key.strip_prefix("to_")?;
    if target_unit.is_empty() {
        return None;
    }
    Some(CustomTimeConversion {
        target_unit: target_unit.to_string(),
        to_target_expr: expr.clone(),
    })
}

fn number_to_f64(value: Number) -> f64 {
    match value {
        Number::Int(i) => i as f64,
        Number::Float(f) => f,
    }
}

fn eval_conversion_expr_with_value(expr: &ast::ConversionExpr, value: f64) -> Option<f64> {
    match expr {
        ast::ConversionExpr::Val => Some(value),
        ast::ConversionExpr::Lit(n) => Some(*n),
        ast::ConversionExpr::BinaryOp { lhs, op, rhs } => {
            let left = eval_conversion_expr_with_value(lhs, value)?;
            let right = eval_conversion_expr_with_value(rhs, value)?;
            match op {
                ast::BinOp::Add => Some(left + right),
                ast::BinOp::Sub => Some(left - right),
                ast::BinOp::Mul => Some(left * right),
                ast::BinOp::Div => {
                    if right == 0.0 {
                        None
                    } else {
                        Some(left / right)
                    }
                }
                _ => None,
            }
        }
        ast::ConversionExpr::Paren(inner) => eval_conversion_expr_with_value(inner, value),
        ast::ConversionExpr::UnaryNeg(inner) => {
            Some(-eval_conversion_expr_with_value(inner, value)?)
        }
    }
}

fn time_value_to_micros(
    value: Number,
    unit: &str,
    custom_time_conversions: &HashMap<String, CustomTimeConversion>,
) -> Option<i64> {
    let mut seen = HashSet::new();
    time_value_to_micros_inner(value, unit, custom_time_conversions, &mut seen)
}

fn time_value_to_micros_inner(
    value: Number,
    unit: &str,
    custom_time_conversions: &HashMap<String, CustomTimeConversion>,
    seen: &mut HashSet<String>,
) -> Option<i64> {
    if let Some(micros) = types::builtin_time_to_micros(value.clone(), unit) {
        return Some(micros);
    }

    if seen.contains(unit) {
        return None;
    }

    let conversion = custom_time_conversions.get(unit)?;
    seen.insert(unit.to_string());

    let converted_value =
        eval_conversion_expr_with_value(&conversion.to_target_expr, number_to_f64(value))?;
    if !converted_value.is_finite() {
        return None;
    }

    let result = time_value_to_micros_inner(
        Number::Float(converted_value),
        &conversion.target_unit,
        custom_time_conversions,
        seen,
    );
    seen.remove(unit);
    result
}

fn lower_sleep_duration(
    value: &Number,
    unit: &str,
    custom_time_conversions: &HashMap<String, CustomTimeConversion>,
) -> Option<i64> {
    time_value_to_micros(value.clone(), unit, custom_time_conversions)
}

fn lower_bin_op(op: &AstBinOp) -> IrBinOp {
    match op {
        AstBinOp::Add => IrBinOp::Add,
        AstBinOp::Sub => IrBinOp::Sub,
        AstBinOp::Mul => IrBinOp::Mul,
        AstBinOp::Div => IrBinOp::Div,
        AstBinOp::Mod => IrBinOp::Mod,
        AstBinOp::Pow => IrBinOp::Pow,
        AstBinOp::Eq => IrBinOp::Eq,
        AstBinOp::Neq => IrBinOp::Neq,
        AstBinOp::Lt => IrBinOp::Lt,
        AstBinOp::Gt => IrBinOp::Gt,
        AstBinOp::Le => IrBinOp::Le,
        AstBinOp::Ge => IrBinOp::Ge,
        AstBinOp::And => IrBinOp::And,
        AstBinOp::Or => IrBinOp::Or,
    }
}

fn lower_un_op(op: &AstUnOp) -> IrUnOp {
    match op {
        AstUnOp::Neg => IrUnOp::Neg,
        AstUnOp::Not => IrUnOp::Not,
    }
}
fn parse_ir_type(s: &str) -> IrType {
    match s {
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "int" => IrType::Int,
        "f32" | "f64" | "float" => IrType::Float,
        "bool" => IrType::Bool,
        "void" => IrType::Void,
        "string" | "str" => IrType::String,
        "Pin" | "pin" => IrType::Pin,
        _ => IrType::Int,
    }
}

pub fn validate_program(program: &IrProgram) -> IrResult<()> {
    if program.tasks.is_empty() && program.setup_body.is_empty() {
        return Err(IrError::NoTasksDefined);
    }

    for task in &program.tasks {
        validate_task(task)?;
    }

    for func in &program.functions {
        validate_function(func)?;
    }

    for stmt in &program.setup_body {
        validate_stmt(stmt)?;
    }

    Ok(())
}

fn validate_task(task: &IrTask) -> IrResult<()> {
    match task.trigger {
        task_ir::TaskTrigger::Periodic {
            period_ticks,
            phase_ticks: _,
        } => {
            if period_ticks == 0 {
                return Err(IrError::InvalidTaskPeriod {
                    period_micros: 0,
                    source_span: task.source_span,
                });
            }
        }
    }

    for stmt in &task.body {
        validate_stmt(stmt).map_err(|err| err.with_fallback_span(task.source_span))?;
    }

    Ok(())
}

fn validate_function(func: &task_ir::IrFunction) -> IrResult<()> {
    for stmt in &func.body {
        validate_stmt(stmt).map_err(|err| err.with_fallback_span(func.source_span))?;
    }
    Ok(())
}

fn validate_stmt(stmt: &IrStmt) -> IrResult<()> {
    let source_span = match stmt {
        IrStmt::Read { source_span, .. }
        | IrStmt::Write { source_span, .. }
        | IrStmt::Sleep { source_span, .. }
        | IrStmt::If { source_span, .. }
        | IrStmt::While { source_span, .. }
        | IrStmt::For { source_span, .. }
        | IrStmt::Assign { source_span, .. }
        | IrStmt::Return { source_span, .. }
        | IrStmt::PeriodicBlock { source_span, .. } => *source_span,
        IrStmt::Expr(_) => None,
    };

    let result = match stmt {
        IrStmt::Read {
            sensor, variable, ..
        } => {
            if sensor.is_empty() {
                Err(IrError::UnknownSensor {
                    name: sensor.clone(),
                    source_span,
                })
            } else if variable.is_empty() {
                Err(IrError::UnknownVariable {
                    name: variable.clone(),
                    source_span,
                })
            } else {
                Ok(())
            }
        }
        IrStmt::Write { output, value, .. } => {
            if output.is_empty() {
                Err(IrError::UnknownVariable {
                    name: output.clone(),
                    source_span,
                })
            } else {
                validate_expr(value)
            }
        }
        IrStmt::Sleep {
            duration_micros, ..
        } => {
            match duration_micros {
                Some(v) if *v > 0 => Ok(()),
                _ => Err(IrError::InvalidSleepDuration {
                    duration_micros: *duration_micros,
                    source_span,
                }),
            }
        }
        IrStmt::PeriodicBlock { body, .. } => {
            for s in body {
                validate_stmt(s)?;
            }
            Ok(())
        }
        IrStmt::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            validate_expr(condition)?;
            for s in then_body {
                validate_stmt(s)?;
            }
            if let Some(else_body) = else_body {
                for s in else_body {
                    validate_stmt(s)?;
                }
            }
            Ok(())
        }
        IrStmt::While {
            condition, body, ..
        } => {
            validate_expr(condition)?;
            for s in body {
                validate_stmt(s)?;
            }
            Ok(())
        }
        IrStmt::For {
            variable,
            iterable,
            body,
            ..
        } => {
            if variable.is_empty() {
                return Err(IrError::UnknownVariable {
                    name: variable.clone(),
                    source_span,
                });
            }
            if !matches!((&iterable.kind, &iterable.ty), (IrExprKind::Array(_), IrType::Array(_))) {
                return Err(IrError::UnsupportedForIterable { source_span });
            }
            validate_expr(iterable)?;
            for s in body {
                validate_stmt(s)?;
            }
            Ok(())
        }
        IrStmt::Assign {
            variable: _, value, ..
        } => validate_expr(value),
        IrStmt::Return { value, .. } => {
            if let Some(v) = value {
                validate_expr(v)
            } else {
                Ok(())
            }
        }
        IrStmt::Expr(expr) => validate_expr(expr),
    };

    result.map_err(|err| err.with_fallback_span(source_span))
}

fn validate_expr(expr: &IrExpr) -> IrResult<()> {
    match &expr.kind {
        IrExprKind::IntLit(_)
        | IrExprKind::FloatLit(_)
        | IrExprKind::BoolLit(_)
        | IrExprKind::StringLit(_) => Ok(()),
        IrExprKind::Array(elements) => {
            for element in elements {
                validate_expr(element)?;
            }
            Ok(())
        }
        IrExprKind::Index { object, index } => {
            validate_expr(object)?;
            validate_expr(index)
        }
        IrExprKind::Field { object, .. } => validate_expr(object),
        IrExprKind::UnitLit { unit, category, .. } => {
            if expr.unit.as_ref() != Some(category) {
                return Err(IrError::TypeMismatch {
                    expected: IrType::Unit(category.clone()),
                    found: expr.ty.clone(),
                    source_span: expr.source_span,
                });
            }

            match &expr.ty {
                IrType::Unit(ty_category) if ty_category == category => {}
                found => {
                    return Err(IrError::TypeMismatch {
                        expected: IrType::Unit(category.clone()),
                        found: found.clone(),
                        source_span: expr.source_span,
                    });
                }
            }

            if unit.is_empty() {
                return Err(IrError::TypeMismatch {
                    expected: IrType::Unit(category.clone()),
                    found: expr.ty.clone(),
                    source_span: expr.source_span,
                });
            }
            Ok(())
        }
        IrExprKind::Ident(_) => Ok(()),
        IrExprKind::BinaryOp { lhs, op, rhs } => {
            if matches!(op, IrBinOp::Div) && rhs.ty.is_numeric() {
                match rhs.kind {
                    IrExprKind::IntLit(0) => {
                        return Err(IrError::DivisionByZero {
                            source_span: expr.source_span,
                        });
                    }
                    IrExprKind::FloatLit(0.0) => {
                        return Err(IrError::DivisionByZero {
                            source_span: expr.source_span,
                        });
                    }
                    _ => {}
                }
            }
            validate_expr(lhs)?;
            validate_expr(rhs)
        }
        IrExprKind::UnaryOp { expr, .. } => validate_expr(expr),
        IrExprKind::Cast { expr, target } => {
            validate_expr(expr)?;
            let cast_allowed = matches!(
                (&expr.ty, target),
                (IrType::Int, IrType::Int)
                    | (IrType::Int, IrType::Float)
                    | (IrType::Int, IrType::Bool)
                    | (IrType::Int, IrType::String)
                    | (IrType::Float, IrType::Int)
                    | (IrType::Float, IrType::Float)
                    | (IrType::Float, IrType::Bool)
                    | (IrType::Float, IrType::String)
                    | (IrType::Bool, IrType::Int)
                    | (IrType::Bool, IrType::Float)
                    | (IrType::Bool, IrType::Bool)
                    | (IrType::Bool, IrType::String)
                    | (IrType::String, IrType::Int)
                    | (IrType::String, IrType::Float)
                    | (IrType::String, IrType::Bool)
                    | (IrType::String, IrType::String)
                    | (IrType::Unit(_), IrType::Int)
                    | (IrType::Unit(_), IrType::Float)
            );
            if cast_allowed {
                Ok(())
            } else {
                Err(IrError::TypeMismatch {
                    expected: target.clone(),
                    found: expr.ty.clone(),
                    source_span: expr.source_span,
                })
            }
        }
        IrExprKind::Call { func, args } => {
            if func.is_empty() {
                return Err(IrError::UnknownVariable {
                    name: "<empty call path>".to_string(),
                    source_span: expr.source_span,
                });
            }
            for arg in args {
                validate_expr(arg)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::{program_parser, token_stream};
    use crate::task_ir::{EnergyPolicy, SchedulerConfig, SleepFallback, TaskTrigger};
    use crate::typechecker::TypeChecker;
    use chumsky::Parser;

    fn parse_typecheck_and_lower(input: &str) -> IrResult<IrProgram> {
        let lexer = Lexer::new();
        let tokens = lexer.tokenize_spanned(input).unwrap();
        let mut program = program_parser()
            .parse(token_stream(&tokens, input.len()))
            .into_result()
            .unwrap();

        let mut checker = TypeChecker::new();
        checker
            .check_program(&mut program)
            .expect("type checking failed unexpectedly");

        lower_program(&program)
    }

    fn parse_and_lower_without_typecheck(input: &str) -> IrResult<IrProgram> {
        let lexer = Lexer::new();
        let tokens = lexer.tokenize_spanned(input).unwrap();
        let program = program_parser()
            .parse(token_stream(&tokens, input.len()))
            .into_result()
            .unwrap();
        lower_program(&program)
    }

    #[test]
    fn test_parse_ir_type() {
        assert_eq!(parse_ir_type("int"), IrType::Int);
        assert_eq!(parse_ir_type("float"), IrType::Float);
        assert_eq!(parse_ir_type("bool"), IrType::Bool);
        assert_eq!(parse_ir_type("void"), IrType::Void);
        assert_eq!(parse_ir_type("unknown"), IrType::Int);
    }

    #[test]
    fn test_lower_bin_op() {
        assert_eq!(lower_bin_op(&AstBinOp::Add), IrBinOp::Add);
        assert_eq!(lower_bin_op(&AstBinOp::Sub), IrBinOp::Sub);
        assert_eq!(lower_bin_op(&AstBinOp::Mul), IrBinOp::Mul);
        assert_eq!(lower_bin_op(&AstBinOp::Div), IrBinOp::Div);
    }

    #[test]
    fn test_lower_un_op() {
        assert_eq!(lower_un_op(&AstUnOp::Neg), IrUnOp::Neg);
        assert_eq!(lower_un_op(&AstUnOp::Not), IrUnOp::Not);
    }

    #[test]
    fn test_calculate_unit_conversions() {
        use crate::ast::ConversionExpr;

        let conversions = vec![
            ("to_base".to_string(), ConversionExpr::Lit(1000.0)),
            ("from_base".to_string(), ConversionExpr::Lit(1.0)),
        ];

        let (to_base, from_base) = calculate_unit_conversions(&conversions).unwrap();
        assert_eq!(to_base, 1000);
        assert_eq!(from_base, 1);
    }

    #[test]
    fn test_lower_range_array_expands_to_int_elements() {
        let expr = Expr {
            kind: ExprKind::RangeArray { start: 0, end: 5 },
            ty: Some(crate::types::Type::Array(Box::new(crate::types::Type::Int))),
            unit: None,
            span: None,
        };

        let lowered = lower_expr(&expr, &IrLoweringContext::new()).unwrap();

        match lowered.kind {
            IrExprKind::Array(elements) => {
                assert_eq!(elements.len(), 5);
                for (expected, element) in (0_i64..5).zip(elements) {
                    assert!(matches!(element.kind, IrExprKind::IntLit(value) if value == expected));
                }
            }
            _ => panic!("Expected lowered range array"),
        }
    }

    #[test]
    fn test_lower_program_accepts_custom_category_unit_definition() {
        let input = r#"
            unit psi : pressure {
                to_pa: val * 6894.76,
                from_pa: val / 6894.76
            }
            every 1s {
                x = 10psi
            }
        "#;

        let ir = parse_typecheck_and_lower(input).expect("lowering should succeed");

        let unit_defs = ir
            .definitions
            .iter()
            .filter_map(|def| match def {
                IrDefinition::Unit(unit) => Some(unit),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(unit_defs.len(), 1);
        assert_eq!(unit_defs[0].name, "psi");
        assert_eq!(
            unit_defs[0].category,
            UnitCategory::Custom("pressure".to_string())
        );
    }

    #[test]
    fn test_lower_program_propagates_typed_sensor_read_metadata() {
        let input = r#"
            fn convert_temp(raw: int) -> float {
                return 0.0
            }
            sensor temp on A0 : temperature using convert_temp
            every 1s {
                read temp -> t
            }
        "#;

        let ir = parse_typecheck_and_lower(input).expect("lowering should succeed");

        let sensor_defs = ir
            .definitions
            .iter()
            .filter_map(|def| match def {
                IrDefinition::Sensor(sensor) => Some(sensor),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(sensor_defs.len(), 1);
        assert_eq!(sensor_defs[0].name, "temp");
        assert_eq!(sensor_defs[0].category, Some(UnitCategory::Temperature));
        assert_eq!(
            sensor_defs[0].converter,
            Some(vec!["convert_temp".to_string()])
        );

        let body = &ir.tasks[0].body;
        match &body[0] {
            IrStmt::PeriodicBlock { body, .. } => match &body[0] {
                IrStmt::Read {
                    value_type,
                    converter,
                    ..
                } => {
                    assert_eq!(*value_type, IrType::Unit(UnitCategory::Temperature));
                    assert_eq!(converter, &Some(vec!["convert_temp".to_string()]));
                }
                other => panic!("expected read statement, got {:?}", other),
            },
            other => panic!("expected periodic block, got {:?}", other),
        }
    }

    #[test]
    fn test_typechecker_rejects_different_custom_categories_in_binary_op() {
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

        let lexer = Lexer::new();
        let tokens = lexer.tokenize_spanned(input).unwrap();
        let mut program = program_parser()
            .parse(token_stream(&tokens, input.len()))
            .into_result()
            .unwrap();

        let mut checker = TypeChecker::new();
        let errs = checker
            .check_program(&mut program)
            .expect_err("expected type checking failure");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("requires the same category"))
        );
    }

    #[test]
    fn test_lower_every_with_custom_time_unit_produces_valid_period_ticks() {
        let input = r#"
            unit tick : time {
                to_ms: val * 10,
                from_ms: val / 10
            }
            every 100tick { }
        "#;

        let ir = parse_typecheck_and_lower(input).expect("lowering should succeed");
        assert_eq!(ir.tasks.len(), 1);
        match ir.tasks[0].trigger {
            task_ir::TaskTrigger::Periodic { period_ticks, .. } => {
                assert_eq!(period_ticks, 1000);
            }
        }
    }

    #[test]
    fn test_lower_sleep_with_custom_time_unit_converts_to_micros() {
        let input = r#"
            unit tick : time {
                to_ms: val * 10,
                from_ms: val / 10
            }
            every 1s {
                sleep 50tick
            }
        "#;

        let ir = parse_typecheck_and_lower(input).expect("lowering should succeed");
        assert_eq!(ir.tasks.len(), 1);
        let body = &ir.tasks[0].body;
        assert_eq!(body.len(), 1);

        match &body[0] {
            IrStmt::PeriodicBlock { body, .. } => {
                assert_eq!(body.len(), 1);
                match &body[0] {
                    IrStmt::Sleep {
                        duration_micros, ..
                    } => {
                        assert_eq!(*duration_micros, Some(500_000));
                    }
                    other => panic!("expected sleep statement, got {:?}", other),
                }
            }
            other => panic!("expected periodic block, got {:?}", other),
        }
    }

    #[test]
    fn lr_invalid_period_cycle_returns_invalid_task_period_with_span() {
        let input = r#"
            unit tick : time {
                to_tock: val,
                from_tock: val
            }
            unit tock : time {
                to_tick: val,
                from_tick: val
            }
            every 1tick { }
        "#;

        let err = parse_and_lower_without_typecheck(input).expect_err("expected lowering failure");
        match err {
            IrError::InvalidTaskPeriod { source_span, .. } => {
                assert!(
                    source_span.is_some(),
                    "expected span on invalid task period"
                );
            }
            other => panic!("expected InvalidTaskPeriod, got {:?}", other),
        }
    }

    #[test]
    fn lr_invalid_sleep_duration_is_error_not_none_payload() {
        let input = r#"
            unit tick : time {
                to_tock: val,
                from_tock: val
            }
            every 1s {
                sleep 1tick
            }
        "#;

        let err = parse_and_lower_without_typecheck(input).expect_err("expected lowering failure");
        match err {
            IrError::InvalidTaskPeriod { source_span, .. } => {
                assert!(
                    source_span.is_some(),
                    "expected span on invalid sleep duration"
                );
            }
            other => panic!("expected InvalidTaskPeriod, got {:?}", other),
        }
    }

    #[test]
    fn lr_typed_sensor_time_read_metadata_propagates() {
        let input = r#"
            fn convert_tick(raw: int) -> int {
                return raw
            }
            sensor tick_sensor on A0 : time using convert_tick
            every 1s {
                read tick_sensor -> t
            }
        "#;

        let ir = parse_typecheck_and_lower(input).expect("lowering should succeed");
        let body = &ir.tasks[0].body;
        match &body[0] {
            IrStmt::PeriodicBlock { body, .. } => match &body[0] {
                IrStmt::Read {
                    value_type,
                    converter,
                    ..
                } => {
                    assert_eq!(*value_type, IrType::Unit(UnitCategory::Time));
                    assert_eq!(converter, &Some(vec!["convert_tick".to_string()]));
                }
                other => panic!("expected read statement, got {:?}", other),
            },
            other => panic!("expected periodic block, got {:?}", other),
        }
    }

    #[test]
    fn lr_non_time_custom_unit_preserves_fractional_value() {
        let input = r#"
            unit psi : pressure {
                to_pa: val * 6894.76,
                from_pa: val / 6894.76
            }
            every 1s {
                p = 1psi
            }
        "#;

        let ir = parse_typecheck_and_lower(input).expect("lowering should succeed");
        let body = &ir.tasks[0].body;
        let IrStmt::PeriodicBlock { body, .. } = &body[0] else {
            panic!("expected periodic block");
        };
        let IrStmt::Assign { value, .. } = &body[0] else {
            panic!("expected assignment");
        };

        let IrExprKind::UnitLit {
            value: lowered_value,
            unit,
            category,
        } = &value.kind
        else {
            panic!("expected lowered unit literal");
        };
        assert_eq!(unit, "pa");
        assert_eq!(category, &UnitCategory::Custom("pressure".to_string()));
        assert!(matches!(lowered_value, Number::Float(v) if (*v - 6894.76).abs() < 1e-6));
    }

    #[test]
    fn lr_task_block_lowers_into_setup_body() {
        let input = r#"
            sensor temp on A0
            task {
                read temp -> t
                sleep 500ms
            }
        "#;

        let ir = parse_typecheck_and_lower(input).expect("lowering should succeed");
        assert!(ir.tasks.is_empty());
        assert_eq!(ir.setup_body.len(), 2);
        match &ir.setup_body[0] {
            IrStmt::Read { variable, .. } => assert_eq!(variable, "t"),
            other => panic!("expected read in setup body, got {:?}", other),
        }
        match &ir.setup_body[1] {
            IrStmt::Sleep {
                duration_micros, ..
            } => assert_eq!(*duration_micros, Some(500_000)),
            other => panic!("expected sleep in setup body, got {:?}", other),
        }
    }

    #[test]
    fn lr_validate_program_accepts_task_only_program() {
        let input = r#"
            task {
                x = 1
            }
        "#;
        let ir = parse_typecheck_and_lower(input).expect("lowering should succeed");
        assert!(ir.tasks.is_empty());
        assert!(!ir.setup_body.is_empty());
        validate_program(&ir).expect("task-only program should validate");
    }

    #[test]
    fn lr_validate_program_rejects_missing_sleep_duration_payload() {
        let program = IrProgram {
            definitions: vec![],
            tasks: vec![IrTask {
                name: None,
                trigger: TaskTrigger::Periodic {
                    period_ticks: 1,
                    phase_ticks: 0,
                },
                body: vec![IrStmt::Sleep {
                    duration_micros: None,
                    mode_hint: None,
                    wake_sources: vec![],
                    fallback: SleepFallback::UseActiveDelay,
                    source_span: None,
                }],
                source_span: None,
            }],
            setup_body: vec![],
            functions: vec![],
            scheduler: SchedulerConfig::default(),
            energy_policy: EnergyPolicy::default(),
        };

        let err = validate_program(&program).expect_err("validation should fail");
        match err {
            IrError::InvalidSleepDuration {
                duration_micros: None,
                ..
            } => {}
            other => panic!("expected InvalidSleepDuration(None), got {:?}", other),
        }
    }

    #[test]
    fn lr_validate_program_rejects_zero_sleep_duration_payload() {
        let program = IrProgram {
            definitions: vec![],
            tasks: vec![IrTask {
                name: None,
                trigger: TaskTrigger::Periodic {
                    period_ticks: 1,
                    phase_ticks: 0,
                },
                body: vec![IrStmt::Sleep {
                    duration_micros: Some(0),
                    mode_hint: None,
                    wake_sources: vec![],
                    fallback: SleepFallback::UseActiveDelay,
                    source_span: None,
                }],
                source_span: None,
            }],
            setup_body: vec![],
            functions: vec![],
            scheduler: SchedulerConfig::default(),
            energy_policy: EnergyPolicy::default(),
        };

        let err = validate_program(&program).expect_err("validation should fail");
        match err {
            IrError::InvalidSleepDuration {
                duration_micros: Some(0),
                ..
            } => {}
            other => panic!("expected InvalidSleepDuration(Some(0)), got {:?}", other),
        }
    }

    #[test]
    fn lr_validate_program_rejects_non_array_for_iterable() {
        let program = IrProgram {
            definitions: vec![],
            tasks: vec![IrTask {
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
            }],
            setup_body: vec![],
            functions: vec![],
            scheduler: SchedulerConfig::default(),
            energy_policy: EnergyPolicy::default(),
        };

        let err = validate_program(&program).expect_err("validation should fail");
        match err {
            IrError::UnsupportedForIterable { .. } => {}
            other => panic!("expected UnsupportedForIterable, got {:?}", other),
        }
    }

    #[test]
    fn lr_validate_program_accepts_explicit_unit_to_float_cast() {
        let program = IrProgram {
            definitions: vec![],
            tasks: vec![IrTask {
                name: None,
                trigger: TaskTrigger::Periodic {
                    period_ticks: 1,
                    phase_ticks: 0,
                },
                body: vec![IrStmt::Assign {
                    variable: "temper".to_string(),
                    value: IrExpr {
                        kind: IrExprKind::Cast {
                            expr: Box::new(IrExpr {
                                kind: IrExprKind::UnitLit {
                                    value: Number::Float(25.0),
                                    unit: "c".to_string(),
                                    category: UnitCategory::Temperature,
                                },
                                ty: IrType::Unit(UnitCategory::Temperature),
                                unit: Some(UnitCategory::Temperature),
                                source_span: None,
                            }),
                            target: IrType::Float,
                        },
                        ty: IrType::Float,
                        unit: None,
                        source_span: None,
                    },
                    source_span: None,
                }],
                source_span: None,
            }],
            setup_body: vec![],
            functions: vec![],
            scheduler: SchedulerConfig::default(),
            energy_policy: EnergyPolicy::default(),
        };

        validate_program(&program).expect("explicit unit-to-float cast should validate");
    }
}
