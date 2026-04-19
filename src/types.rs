use crate::ast::Number;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnitCategory {
    Time,
    Temperature,
    Distance,
    Voltage,
    Percentage,
    Frequency,
    Custom(String),
}

pub fn is_valid_category_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }

    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

pub fn parse_category(s: &str) -> Option<UnitCategory> {
    match s {
        "time" => Some(UnitCategory::Time),
        "temperature" => Some(UnitCategory::Temperature),
        "distance" => Some(UnitCategory::Distance),
        "voltage" => Some(UnitCategory::Voltage),
        "percentage" => Some(UnitCategory::Percentage),
        "frequency" => Some(UnitCategory::Frequency),
        _ if is_valid_category_name(s) => Some(UnitCategory::Custom(s.to_string())),
        _ => None,
    }
}

pub fn builtin_time_to_micros(value: Number, unit: &str) -> Option<i64> {
    match value {
        Number::Int(i) => match unit {
            "us" | "μs" => Some(i),
            "ms" => Some(i.checked_mul(1_000)?),
            "s" => Some(i.checked_mul(1_000_000)?),
            "min" => Some(i.checked_mul(60_000_000)?),
            "h" => Some(i.checked_mul(3_600_000_000)?),
            _ => None,
        },
        Number::Float(f) => {
            let micros = match unit {
                "us" | "μs" => Some(f),
                "ms" => Some(f * 1_000.0),
                "s" => Some(f * 1_000_000.0),
                "min" => Some(f * 60_000_000.0),
                "h" => Some(f * 3_600_000_000.0),
                _ => None,
            }?;

            Some(micros.round() as i64)
        }
    }
}

pub fn categorize_builtin_unit(unit: &str) -> Option<UnitCategory> {
    match unit {
        "ms" | "s" | "min" | "h" | "us" | "μs" => Some(UnitCategory::Time),
        "c" | "f" | "k" => Some(UnitCategory::Temperature),
        "mm" | "cm" | "m" | "km" => Some(UnitCategory::Distance),
        "mv" | "v" => Some(UnitCategory::Voltage),
        "%" => Some(UnitCategory::Percentage),
        "hz" | "khz" | "mhz" => Some(UnitCategory::Frequency),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CustomUnitDef {
    pub name: String,
    pub category: UnitCategory,
    pub to_base: crate::ast::ConversionExpr,
    pub from_base: crate::ast::ConversionExpr,
}

#[derive(Debug, Clone, Default)]
pub struct UnitRegistry {
    pub custom_units: std::collections::HashMap<String, CustomUnitDef>,
}

impl UnitRegistry {
    pub fn new() -> Self {
        UnitRegistry {
            custom_units: std::collections::HashMap::new(),
        }
    }

    pub fn register(&mut self, def: CustomUnitDef) -> Result<(), String> {
        if categorize_builtin_unit(&def.name).is_some() {
            return Err(format!(
                "Unit '{}' conflicts with a built-in unit",
                def.name
            ));
        }
        if self.custom_units.contains_key(&def.name) {
            return Err(format!("Unit '{}' is already defined", def.name));
        }
        self.custom_units.insert(def.name.clone(), def);
        Ok(())
    }

    pub fn categorize(&self, unit: &str) -> Option<UnitCategory> {
        if let Some(cat) = categorize_builtin_unit(unit) {
            return Some(cat);
        }
        self.custom_units.get(unit).map(|def| def.category.clone())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Float,
    Bool,
    String,
    Unit(UnitCategory),
    Pin,
    Sensor,
    Output,
    Array(Box<Type>),
    Function(Vec<Type>, Box<Type>),
    Void,
    Unknown,
}

impl std::fmt::Display for UnitCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnitCategory::Time => write!(f, "time"),
            UnitCategory::Temperature => write!(f, "temperature"),
            UnitCategory::Distance => write!(f, "distance"),
            UnitCategory::Voltage => write!(f, "voltage"),
            UnitCategory::Percentage => write!(f, "percentage"),
            UnitCategory::Frequency => write!(f, "frequency"),
            UnitCategory::Custom(name) => write!(f, "{}", name),
        }
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Int => write!(f, "int"),
            Type::Float => write!(f, "float"),
            Type::Bool => write!(f, "bool"),
            Type::String => write!(f, "string"),
            Type::Unit(cat) => write!(f, "{}", cat),
            Type::Pin => write!(f, "pin"),
            Type::Sensor => write!(f, "sensor"),
            Type::Output => write!(f, "output"),
            Type::Array(t) => write!(f, "[]{}", t),
            Type::Function(params, ret) => {
                let params_str = params
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "fn({}) -> {}", params_str, ret)
            }
            Type::Void => write!(f, "void"),
            Type::Unknown => write!(f, "unknown"),
        }
    }
}

pub fn parse_type_name(s: &str) -> Option<Type> {
    match s {
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "int" => Some(Type::Int),
        "f32" | "f64" | "float" => Some(Type::Float),
        "bool" => Some(Type::Bool),
        "string" | "str" => Some(Type::String),
        "void" => Some(Type::Void),
        "Pin" | "pin" => Some(Type::Pin),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_category_builtin_is_first_class_variant() {
        assert_eq!(parse_category("time"), Some(UnitCategory::Time));
        assert_eq!(
            parse_category("temperature"),
            Some(UnitCategory::Temperature)
        );
    }

    #[test]
    fn test_parse_category_custom_accepts_valid_lowercase_identifier() {
        assert_eq!(
            parse_category("pressure"),
            Some(UnitCategory::Custom("pressure".to_string()))
        );
        assert_eq!(
            parse_category("gas_mix_2"),
            Some(UnitCategory::Custom("gas_mix_2".to_string()))
        );
    }

    #[test]
    fn test_parse_category_rejects_invalid_names() {
        assert_eq!(parse_category("Pressure"), None);
        assert_eq!(parse_category("_pressure"), None);
        assert_eq!(parse_category("pressure-psi"), None);
    }

    #[test]
    fn test_custom_category_equality_and_display_are_exact() {
        let lhs = UnitCategory::Custom("pressure".to_string());
        let same = UnitCategory::Custom("pressure".to_string());
        let different = UnitCategory::Custom("Pressure".to_string());

        assert_eq!(lhs, same);
        assert_ne!(lhs, different);
        assert_eq!(lhs.to_string(), "pressure");
    }
}
