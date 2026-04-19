use serde::Deserialize;
use std::fs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetArch {
    Avr,
    Arm,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompilerConfig {
    pub arch: TargetArch,
    pub clock_hz: u32,
    #[serde(default)]
    pub c_includes: Vec<String>,
    #[serde(default)]
    pub c_prelude: Vec<String>,
}

impl CompilerConfig {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let config: CompilerConfig = toml::from_str(&content)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::{CompilerConfig, TargetArch};

    #[test]
    fn test_config_defaults_optional_codegen_hooks() {
        let config: CompilerConfig = toml::from_str(
            r#"
                arch = "avr"
                clock_hz = 16000000
                c_includes = ["<Arduino.h>"]
            "#,
        )
        .unwrap();

        assert_eq!(config.arch, TargetArch::Avr);
        assert!(config.c_prelude.is_empty());
    }

    #[test]
    fn test_config_parses_codegen_prelude() {
        let config: CompilerConfig = toml::from_str(
            r##"
                arch = "arm"
                clock_hz = 133000000
                c_includes = ["<Arduino.h>", "\"DHT.h\""]
                c_prelude = [
                    "#define DHTPIN 2",
                    "#define DHTTYPE DHT22",
                    "DHT dht(DHTPIN, DHTTYPE);",
                ]
            "##,
        )
        .unwrap();

        assert_eq!(config.arch, TargetArch::Arm);
        assert_eq!(config.c_prelude.len(), 3);
    }
}
