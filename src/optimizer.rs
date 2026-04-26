use crate::ast::*;
use crate::types::Type;

pub struct ConstantFolder;

impl ConstantFolder {
    pub fn new() -> Self {
        Self
    }

    pub fn fold_program(&mut self, program: &mut Program) {
        for stmt in &mut program.statements {
            self.fold_toplevel(stmt);
        }
    }

    fn fold_toplevel(&mut self, toplevel: &mut TopLevel) {
        match toplevel {
            TopLevel::FuncDef(func_def) => {
                for stmt in &mut func_def.body {
                    self.fold_statement(stmt);
                }
            }
            TopLevel::Every(every_block) => {
                for stmt in &mut every_block.body {
                    self.fold_statement(stmt);
                }
            }
            TopLevel::Task(task_block) => {
                for stmt in &mut task_block.body {
                    self.fold_statement(stmt);
                }
            }

            TopLevel::SensorDef(_)
            | TopLevel::OutputDef(_)
            | TopLevel::UnitDef(_)
            | TopLevel::Extern(_) => {}
        }
    }

    fn fold_statement(&mut self, stmt: &mut Statement) {
        let stmt_span = stmt.span();
        match stmt {
            Statement::Expr(expr) => {
                *expr = self.fold_expr(expr.clone());
            }
            Statement::Assignment {
                variable: _, value, ..
            } => {
                *value = self.fold_expr(value.clone());
            }
            Statement::Write { value, .. } => {
                *value = self.fold_expr(value.clone());
            }
            Statement::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                *condition = self.fold_expr(condition.clone());

                if let ExprKind::BoolLit(constant_value) = &condition.kind {
                    if *constant_value {
                        for s in then_body {
                            self.fold_statement(s);
                        }
                        *stmt = Statement::Expr(Expr {
                            kind: ExprKind::Paren(Box::new(Expr {
                                kind: ExprKind::BoolLit(true),
                                ty: Some(Type::Bool),
                                unit: None,
                                span: stmt_span,
                            })),
                            ty: Some(Type::Bool),
                            unit: None,
                            span: stmt_span,
                        });
                    } else if let Some(else_stmts) = else_body {
                        for s in else_stmts {
                            self.fold_statement(s);
                        }
                        *stmt = Statement::Expr(Expr {
                            kind: ExprKind::Paren(Box::new(Expr {
                                kind: ExprKind::BoolLit(true),
                                ty: Some(Type::Bool),
                                unit: None,
                                span: stmt_span,
                            })),
                            ty: Some(Type::Bool),
                            unit: None,
                            span: stmt_span,
                        });
                    } else {
                        *stmt = Statement::Expr(Expr {
                            kind: ExprKind::Paren(Box::new(Expr {
                                kind: ExprKind::BoolLit(true),
                                ty: Some(Type::Bool),
                                unit: None,
                                span: stmt_span,
                            })),
                            ty: Some(Type::Bool),
                            unit: None,
                            span: stmt_span,
                        });
                    }
                } else {
                    for s in then_body {
                        self.fold_statement(s);
                    }
                    if let Some(else_stmts) = else_body {
                        for s in else_stmts {
                            self.fold_statement(s);
                        }
                    }
                }
            }
            Statement::While {
                condition, body, ..
            } => {
                *condition = self.fold_expr(condition.clone());
                for s in body {
                    self.fold_statement(s);
                }
            }
            Statement::For {
                variable: _,
                iterable,
                body,
                ..
            } => {
                *iterable = self.fold_expr(iterable.clone());
                for s in body {
                    self.fold_statement(s);
                }
            }
            Statement::Return {
                value: opt_expr, ..
            } => {
                if let Some(expr) = opt_expr {
                    *expr = self.fold_expr(expr.clone());
                }
            }
            Statement::Read { .. } | Statement::Sleep { .. } => {}
        }
    }

    fn fold_expr(&mut self, expr: Expr) -> Expr {
        let folded_expr = match expr.kind {
            ExprKind::BinaryOp { lhs, op, rhs } => {
                let lhs = Box::new(self.fold_expr(*lhs));
                let rhs = Box::new(self.fold_expr(*rhs));
                Expr {
                    kind: ExprKind::BinaryOp { lhs, op, rhs },
                    ..expr
                }
            }
            ExprKind::UnaryOp { op, expr: inner } => {
                let inner = Box::new(self.fold_expr(*inner));
                Expr {
                    kind: ExprKind::UnaryOp { op, expr: inner },
                    ..expr
                }
            }
            ExprKind::Cast {
                expr: inner,
                target,
            } => {
                let inner = Box::new(self.fold_expr(*inner));
                Expr {
                    kind: ExprKind::Cast {
                        expr: inner,
                        target,
                    },
                    ..expr
                }
            }
            ExprKind::RangeArray { start, end } => Expr {
                kind: ExprKind::RangeArray { start, end },
                ..expr
            },
            ExprKind::Paren(inner) => {
                let inner = Box::new(self.fold_expr(*inner));
                Expr {
                    kind: ExprKind::Paren(inner),
                    ..expr
                }
            }
            ExprKind::Array(mut exprs) => {
                for e in &mut exprs {
                    *e = self.fold_expr(e.clone());
                }
                Expr {
                    kind: ExprKind::Array(exprs),
                    ..expr
                }
            }
            ExprKind::Index { object, index } => {
                let object = Box::new(self.fold_expr(*object));
                let index = Box::new(self.fold_expr(*index));
                Expr {
                    kind: ExprKind::Index { object, index },
                    ..expr
                }
            }
            ExprKind::Call { func, mut args } => {
                let func = Box::new(self.fold_expr(*func));
                for arg in &mut args {
                    *arg = self.fold_expr(arg.clone());
                }
                Expr {
                    kind: ExprKind::Call { func, args },
                    ..expr
                }
            }
            ExprKind::Field { object, field } => {
                let object = Box::new(self.fold_expr(*object));
                Expr {
                    kind: ExprKind::Field { object, field },
                    ..expr
                }
            }
            _ => expr,
        };

        self.try_fold_top_level(&folded_expr)
    }

    fn try_fold_top_level(&self, expr: &Expr) -> Expr {
        match &expr.kind {
            ExprKind::BinaryOp { lhs, op, rhs } => self.fold_binary(expr, lhs, op, rhs),
            ExprKind::Paren(inner) => match inner.kind {
                ExprKind::IntLit(_) | ExprKind::FloatLit(_) | ExprKind::BoolLit(_) => {
                    *inner.clone()
                }
                _ => expr.clone(),
            },
            _ => expr.clone(),
        }
    }

    fn fold_binary(&self, original: &Expr, lhs: &Expr, op: &BinOp, rhs: &Expr) -> Expr {
        match (op, &lhs.kind, &rhs.kind) {
            (BinOp::Add, ExprKind::IntLit(l), ExprKind::IntLit(r)) => {
                self.make_int(l + r, original)
            }
            (BinOp::Sub, ExprKind::IntLit(l), ExprKind::IntLit(r)) => {
                self.make_int(l - r, original)
            }
            (BinOp::Mul, ExprKind::IntLit(l), ExprKind::IntLit(r)) => {
                self.make_int(l * r, original)
            }
            (BinOp::Div, ExprKind::IntLit(l), ExprKind::IntLit(r)) => {
                if *r != 0 {
                    self.make_int(l / r, original)
                } else {
                    original.clone()
                }
            }
            (BinOp::Mod, ExprKind::IntLit(l), ExprKind::IntLit(r)) => {
                if *r != 0 {
                    self.make_int(l % r, original)
                } else {
                    original.clone()
                }
            }

            (BinOp::Add, ExprKind::FloatLit(l), ExprKind::FloatLit(r)) => {
                self.make_float(l + r, original)
            }
            (BinOp::Sub, ExprKind::FloatLit(l), ExprKind::FloatLit(r)) => {
                self.make_float(l - r, original)
            }
            (BinOp::Mul, ExprKind::FloatLit(l), ExprKind::FloatLit(r)) => {
                self.make_float(l * r, original)
            }
            (BinOp::Div, ExprKind::FloatLit(l), ExprKind::FloatLit(r)) => {
                self.make_float(l / r, original)
            }

            (
                op,
                ExprKind::UnitLit {
                    value: l_val,
                    unit: l_unit,
                },
                ExprKind::UnitLit {
                    value: r_val,
                    unit: r_unit,
                },
            ) => match op {
                BinOp::Add | BinOp::Sub => {
                    if l_unit == r_unit {
                        match op {
                            BinOp::Add => match (l_val, r_val) {
                                (Number::Int(l), Number::Int(r)) => {
                                    self.make_unit_int(l + r, l_unit, original)
                                }
                                (Number::Float(l), Number::Float(r)) => {
                                    self.make_unit_float(l + r, l_unit, original)
                                }
                                (Number::Int(l), Number::Float(r)) => {
                                    self.make_unit_float(*l as f64 + r, l_unit, original)
                                }
                                (Number::Float(l), Number::Int(r)) => {
                                    self.make_unit_float(l + *r as f64, l_unit, original)
                                }
                            },
                            BinOp::Sub => match (l_val, r_val) {
                                (Number::Int(l), Number::Int(r)) => {
                                    self.make_unit_int(l - r, l_unit, original)
                                }
                                (Number::Float(l), Number::Float(r)) => {
                                    self.make_unit_float(l - r, l_unit, original)
                                }
                                (Number::Int(l), Number::Float(r)) => {
                                    self.make_unit_float(*l as f64 - r, l_unit, original)
                                }
                                (Number::Float(l), Number::Int(r)) => {
                                    self.make_unit_float(l - *r as f64, l_unit, original)
                                }
                            },
                            _ => original.clone(),
                        }
                    } else {
                        original.clone()
                    }
                }

                BinOp::Mul | BinOp::Div => original.clone(),
                _ => original.clone(),
            },

            (BinOp::Mul, ExprKind::UnitLit { value, unit }, ExprKind::IntLit(scalar)) => {
                match value {
                    Number::Int(i) => self.make_unit_int(i * scalar, unit, original),
                    Number::Float(f) => self.make_unit_float(f * (*scalar as f64), unit, original),
                }
            }
            (BinOp::Mul, ExprKind::UnitLit { value, unit }, ExprKind::FloatLit(scalar)) => {
                match value {
                    Number::Int(i) => self.make_unit_float((*i as f64) * scalar, unit, original),
                    Number::Float(f) => self.make_unit_float(f * scalar, unit, original),
                }
            }

            (BinOp::Mul, ExprKind::IntLit(scalar), ExprKind::UnitLit { value, unit }) => {
                match value {
                    Number::Int(i) => self.make_unit_int(scalar * i, unit, original),
                    Number::Float(f) => self.make_unit_float((*scalar as f64) * f, unit, original),
                }
            }
            (BinOp::Mul, ExprKind::FloatLit(scalar), ExprKind::UnitLit { value, unit }) => {
                match value {
                    Number::Int(i) => self.make_unit_float(scalar * (*i as f64), unit, original),
                    Number::Float(f) => self.make_unit_float(scalar * f, unit, original),
                }
            }

            (BinOp::Div, ExprKind::UnitLit { value, unit }, ExprKind::IntLit(scalar)) => {
                if *scalar == 0 {
                    return original.clone();
                }
                match value {
                    Number::Int(i) => {
                        if i % scalar == 0 {
                            self.make_unit_int(i / scalar, unit, original)
                        } else {
                            self.make_unit_float(*i as f64 / *scalar as f64, unit, original)
                        }
                    }
                    Number::Float(f) => self.make_unit_float(f / (*scalar as f64), unit, original),
                }
            }
            (BinOp::Div, ExprKind::UnitLit { value, unit }, ExprKind::FloatLit(scalar)) => {
                match value {
                    Number::Int(i) => self.make_unit_float((*i as f64) / scalar, unit, original),
                    Number::Float(f) => self.make_unit_float(f / scalar, unit, original),
                }
            }

            (BinOp::Add, _, ExprKind::IntLit(0)) => lhs.clone(),
            (BinOp::Add, _, ExprKind::FloatLit(f)) if *f == 0.0 => lhs.clone(),

            (BinOp::Add, ExprKind::IntLit(0), _) => rhs.clone(),
            (BinOp::Add, ExprKind::FloatLit(f), _) if *f == 0.0 => rhs.clone(),

            (BinOp::Mul, _, ExprKind::IntLit(1)) => lhs.clone(),
            (BinOp::Mul, _, ExprKind::FloatLit(f)) if *f == 1.0 => lhs.clone(),

            (BinOp::Mul, ExprKind::IntLit(1), _) => rhs.clone(),
            (BinOp::Mul, ExprKind::FloatLit(f), _) if *f == 1.0 => rhs.clone(),

            (BinOp::Sub, _, ExprKind::IntLit(0)) => lhs.clone(),
            (BinOp::Sub, _, ExprKind::FloatLit(f)) if *f == 0.0 => lhs.clone(),

            (BinOp::Div, _, ExprKind::IntLit(1)) => lhs.clone(),
            (BinOp::Div, _, ExprKind::FloatLit(f)) if *f == 1.0 => lhs.clone(),

            (BinOp::Pow, ExprKind::IntLit(l), ExprKind::IntLit(r)) => {
                if *r < 0 {
                    self.make_float((*l as f64).powf(*r as f64), original)
                } else {
                    self.make_int(l.pow(*r as u32), original)
                }
            }
            (BinOp::Pow, ExprKind::FloatLit(l), ExprKind::FloatLit(r)) => {
                self.make_float(l.powf(*r), original)
            }
            (BinOp::Pow, ExprKind::FloatLit(l), ExprKind::IntLit(r)) => {
                self.make_float(l.powf(*r as f64), original)
            }
            (BinOp::Pow, ExprKind::IntLit(l), ExprKind::FloatLit(r)) => {
                self.make_float((*l as f64).powf(*r), original)
            }

            (BinOp::Eq, ExprKind::IntLit(l), ExprKind::IntLit(r)) => {
                self.make_bool(l == r, original)
            }
            (BinOp::Eq, ExprKind::FloatLit(l), ExprKind::FloatLit(r)) => {
                self.make_bool((*l - *r).abs() < f64::EPSILON, original)
            }
            (BinOp::Eq, ExprKind::BoolLit(l), ExprKind::BoolLit(r)) => {
                self.make_bool(l == r, original)
            }
            (BinOp::Neq, ExprKind::IntLit(l), ExprKind::IntLit(r)) => {
                self.make_bool(l != r, original)
            }
            (BinOp::Neq, ExprKind::FloatLit(l), ExprKind::FloatLit(r)) => {
                self.make_bool((*l - *r).abs() >= f64::EPSILON, original)
            }
            (BinOp::Neq, ExprKind::BoolLit(l), ExprKind::BoolLit(r)) => {
                self.make_bool(l != r, original)
            }
            (BinOp::Lt, ExprKind::IntLit(l), ExprKind::IntLit(r)) => {
                self.make_bool(l < r, original)
            }
            (BinOp::Lt, ExprKind::FloatLit(l), ExprKind::FloatLit(r)) => {
                self.make_bool(l < r, original)
            }
            (BinOp::Gt, ExprKind::IntLit(l), ExprKind::IntLit(r)) => {
                self.make_bool(l > r, original)
            }
            (BinOp::Gt, ExprKind::FloatLit(l), ExprKind::FloatLit(r)) => {
                self.make_bool(l > r, original)
            }
            (BinOp::Le, ExprKind::IntLit(l), ExprKind::IntLit(r)) => {
                self.make_bool(l <= r, original)
            }
            (BinOp::Le, ExprKind::FloatLit(l), ExprKind::FloatLit(r)) => {
                self.make_bool(l <= r, original)
            }
            (BinOp::Ge, ExprKind::IntLit(l), ExprKind::IntLit(r)) => {
                self.make_bool(l >= r, original)
            }
            (BinOp::Ge, ExprKind::FloatLit(l), ExprKind::FloatLit(r)) => {
                self.make_bool(l >= r, original)
            }

            (BinOp::And, ExprKind::BoolLit(l), ExprKind::BoolLit(r)) => {
                self.make_bool(*l && *r, original)
            }
            (BinOp::Or, ExprKind::BoolLit(l), ExprKind::BoolLit(r)) => {
                self.make_bool(*l || *r, original)
            }

            (BinOp::Mul, _, ExprKind::IntLit(0)) => {
                if self.is_safe_to_fold_to_zero(lhs) {
                    match original.ty {
                        Some(Type::Int) => self.make_int(0, original),
                        Some(Type::Float) => self.make_float(0.0, original),
                        _ => original.clone(),
                    }
                } else {
                    original.clone()
                }
            }
            (BinOp::Mul, ExprKind::IntLit(0), _) => {
                if self.is_safe_to_fold_to_zero(rhs) {
                    match original.ty {
                        Some(Type::Int) => self.make_int(0, original),
                        Some(Type::Float) => self.make_float(0.0, original),
                        _ => original.clone(),
                    }
                } else {
                    original.clone()
                }
            }

            _ => original.clone(),
        }
    }

    fn make_int(&self, val: i64, original: &Expr) -> Expr {
        Expr {
            kind: ExprKind::IntLit(val),
            ty: original.ty.clone(),
            unit: original.unit.clone(),
            span: original.span,
        }
    }

    fn make_float(&self, val: f64, original: &Expr) -> Expr {
        Expr {
            kind: ExprKind::FloatLit(val),
            ty: original.ty.clone(),
            unit: original.unit.clone(),
            span: original.span,
        }
    }

    fn make_bool(&self, val: bool, _original: &Expr) -> Expr {
        Expr {
            kind: ExprKind::BoolLit(val),
            ty: Some(Type::Bool),
            unit: None,
            span: _original.span,
        }
    }

    fn make_unit_int(&self, val: i64, unit: &str, original: &Expr) -> Expr {
        Expr {
            kind: ExprKind::UnitLit {
                value: Number::Int(val),
                unit: unit.to_string(),
            },
            ty: original.ty.clone(),
            unit: original.unit.clone(),
            span: original.span,
        }
    }

    fn make_unit_float(&self, val: f64, unit: &str, original: &Expr) -> Expr {
        Expr {
            kind: ExprKind::UnitLit {
                value: Number::Float(val),
                unit: unit.to_string(),
            },
            ty: original.ty.clone(),
            unit: original.unit.clone(),
            span: original.span,
        }
    }

    fn is_safe_to_fold_to_zero(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Call { .. } => false,
            ExprKind::BinaryOp { lhs, rhs, .. } => {
                self.is_safe_to_fold_to_zero(lhs) && self.is_safe_to_fold_to_zero(rhs)
            }
            ExprKind::UnaryOp { expr, .. } => self.is_safe_to_fold_to_zero(expr),
            ExprKind::Cast { expr, .. } => self.is_safe_to_fold_to_zero(expr),
            ExprKind::RangeArray { .. } => true,
            ExprKind::Paren(inner) => self.is_safe_to_fold_to_zero(inner),
            ExprKind::Array(exprs) => exprs.iter().all(|e| self.is_safe_to_fold_to_zero(e)),
            ExprKind::Index { object, index } => {
                self.is_safe_to_fold_to_zero(object) && self.is_safe_to_fold_to_zero(index)
            }
            ExprKind::Field { object, .. } => self.is_safe_to_fold_to_zero(object),

            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::UnitCategory;

    fn int_lit(value: i64) -> Expr {
        Expr {
            kind: ExprKind::IntLit(value),
            ty: Some(Type::Int),
            unit: None,
            span: None,
        }
    }

    fn float_lit(value: f64) -> Expr {
        Expr {
            kind: ExprKind::FloatLit(value),
            ty: Some(Type::Float),
            unit: None,
            span: None,
        }
    }

    fn unit_lit_int(value: i64, unit: &str) -> Expr {
        Expr {
            kind: ExprKind::UnitLit {
                value: Number::Int(value),
                unit: unit.to_string(),
            },
            ty: Some(Type::Unit(UnitCategory::Distance)),
            unit: Some(UnitCategory::Distance),
            span: None,
        }
    }

    fn fold_bin(lhs: Expr, op: BinOp, rhs: Expr) -> Expr {
        let folder = ConstantFolder::new();
        let original = Expr {
            kind: ExprKind::BinaryOp {
                lhs: Box::new(lhs.clone()),
                op: op.clone(),
                rhs: Box::new(rhs.clone()),
            },
            ty: lhs.ty.clone().or(rhs.ty.clone()),
            unit: lhs.unit.clone().or(rhs.unit.clone()),
            span: None,
        };
        folder.fold_binary(&original, &lhs, &op, &rhs)
    }

    #[test]
    fn folds_plain_integer_arithmetic() {
        let folded = fold_bin(int_lit(2), BinOp::Mul, int_lit(3));
        assert_eq!(folded.kind, ExprKind::IntLit(6));
        assert_eq!(folded.ty, Some(Type::Int));
    }

    #[test]
    fn folds_plain_float_arithmetic() {
        let folded = fold_bin(float_lit(10.0), BinOp::Div, float_lit(2.0));
        assert_eq!(folded.kind, ExprKind::FloatLit(5.0));
        assert_eq!(folded.ty, Some(Type::Float));
    }

    #[test]
    fn folds_identity_add_zero() {
        let folded = fold_bin(int_lit(7), BinOp::Add, int_lit(0));
        assert_eq!(folded.kind, ExprKind::IntLit(7));
    }

    #[test]
    fn folds_identity_mul_one() {
        let folded = fold_bin(int_lit(9), BinOp::Mul, int_lit(1));
        assert_eq!(folded.kind, ExprKind::IntLit(9));
    }

    #[test]
    fn folds_same_unit_addition() {
        let folded = fold_bin(unit_lit_int(5, "m"), BinOp::Add, unit_lit_int(3, "m"));
        assert_eq!(
            folded.kind,
            ExprKind::UnitLit {
                value: Number::Int(8),
                unit: "m".to_string()
            }
        );
    }

    #[test]
    fn folds_same_unit_subtraction() {
        let folded = fold_bin(unit_lit_int(10, "s"), BinOp::Sub, unit_lit_int(3, "s"));
        assert_eq!(
            folded.kind,
            ExprKind::UnitLit {
                value: Number::Int(7),
                unit: "s".to_string()
            }
        );
    }

    #[test]
    fn leaves_unit_unit_division_unfolded() {
        let folded = fold_bin(unit_lit_int(10, "m"), BinOp::Div, unit_lit_int(2, "m"));
        assert!(matches!(
            folded.kind,
            ExprKind::BinaryOp { op: BinOp::Div, .. }
        ));
    }

    #[test]
    fn folds_unit_scaling_by_integer() {
        let folded = fold_bin(unit_lit_int(500, "ms"), BinOp::Mul, int_lit(2));
        assert_eq!(
            folded.kind,
            ExprKind::UnitLit {
                value: Number::Int(1000),
                unit: "ms".to_string()
            }
        );
    }

    #[test]
    fn folds_integer_scaling_by_unit() {
        let folded = fold_bin(int_lit(2), BinOp::Mul, unit_lit_int(500, "ms"));
        assert_eq!(
            folded.kind,
            ExprKind::UnitLit {
                value: Number::Int(1000),
                unit: "ms".to_string()
            }
        );
    }

    #[test]
    fn leaves_derived_unit_multiplication_unfolded() {
        let folded = fold_bin(unit_lit_int(5, "m/s"), BinOp::Mul, unit_lit_int(2, "s"));
        assert!(matches!(
            folded.kind,
            ExprKind::BinaryOp { op: BinOp::Mul, .. }
        ));
    }

    #[test]
    fn leaves_derived_unit_division_unfolded() {
        let folded = fold_bin(unit_lit_int(10, "m"), BinOp::Div, unit_lit_int(2, "s"));
        assert!(matches!(
            folded.kind,
            ExprKind::BinaryOp { op: BinOp::Div, .. }
        ));
    }

    #[test]
    fn keeps_side_effecting_zero_multiplication() {
        let call = Expr {
            kind: ExprKind::Call {
                func: Box::new(Expr {
                    kind: ExprKind::Ident("foo".to_string()),
                    ty: None,
                    unit: None,
                    span: None,
                }),
                args: vec![],
            },
            ty: Some(Type::Int),
            unit: None,
            span: None,
        };

        let folded = fold_bin(call.clone(), BinOp::Mul, int_lit(0));
        assert_eq!(
            folded.kind,
            ExprKind::BinaryOp {
                lhs: Box::new(call.clone()),
                op: BinOp::Mul,
                rhs: Box::new(int_lit(0)),
            }
        );
    }

    #[test]
    fn folds_program_recursively() {
        let mut program = Program {
            statements: vec![TopLevel::Every(EveryBlock {
                interval_value: Number::Int(5),
                interval_unit: "s".to_string(),
                body: vec![Statement::Assignment {
                    variable: "x".to_string(),
                    value: Expr {
                        kind: ExprKind::BinaryOp {
                            lhs: Box::new(int_lit(2)),
                            op: BinOp::Mul,
                            rhs: Box::new(int_lit(3)),
                        },
                        ty: Some(Type::Int),
                        unit: None,
                        span: None,
                    },
                    span: None,
                }],
                span: None,
            })],
        };

        ConstantFolder::new().fold_program(&mut program);

        match &program.statements[0] {
            TopLevel::Every(block) => match &block.body[0] {
                Statement::Assignment { value, .. } => {
                    assert_eq!(value.kind, ExprKind::IntLit(6));
                }
                _ => panic!("expected assignment"),
            },
            _ => panic!("expected every block"),
        }
    }
}
