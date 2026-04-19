pub mod arm;
pub mod avr;
pub mod utils;

use crate::config::{CompilerConfig, TargetArch};
use crate::task_ir::IrProgram;

pub fn generate_c_code(program: &IrProgram, config: &CompilerConfig) -> String {
    let mut code = String::new();

    match config.arch {
        TargetArch::Avr => {
            code.push_str(&avr::generate(program, config));
        }
        TargetArch::Arm => {
            code.push_str(&arm::generate(program, config));
        }
    }

    code
}

#[cfg(test)]
mod tests {
    use super::generate_c_code;
    use crate::config::{CompilerConfig, TargetArch};
    use crate::task_ir::{
        EnergyPolicy, IrProgram, IrStmt, PowerState, SchedulerConfig, SleepFallback, WakeSource,
    };

    fn empty_program() -> IrProgram {
        IrProgram {
            definitions: vec![],
            tasks: vec![],
            setup_body: vec![],
            functions: vec![],
            scheduler: SchedulerConfig::default(),
            energy_policy: EnergyPolicy::default(),
        }
    }

    #[test]
    fn test_generate_c_code_includes_configured_prelude() {
        for arch in [TargetArch::Avr, TargetArch::Arm] {
            let config = CompilerConfig {
                arch,
                clock_hz: 16_000_000,
                c_includes: vec!["\"DHT.h\"".to_string()],
                c_prelude: vec![
                    "#define DHTPIN 2".to_string(),
                    "#define DHTTYPE DHT22".to_string(),
                    "DHT dht(DHTPIN, DHTTYPE);".to_string(),
                ],
            };

            let code = generate_c_code(&empty_program(), &config);

            assert!(code.contains("#include \"DHT.h\""));
            assert!(code.contains("#define DHTPIN 2"));
            assert!(code.contains("DHT dht(DHTPIN, DHTTYPE);"));
        }
    }

    #[test]
    fn test_generate_c_code_uses_low_power_sleep_helper_for_explicit_sleep() {
        let program = IrProgram {
            definitions: vec![],
            tasks: vec![],
            setup_body: vec![IrStmt::Sleep {
                duration_micros: Some(250_000),
                mode_hint: Some(PowerState::Idle),
                wake_sources: vec![WakeSource::Timer],
                fallback: SleepFallback::UseActiveDelay,
                source_span: None,
            }],
            functions: vec![],
            scheduler: SchedulerConfig::default(),
            energy_policy: EnergyPolicy::default(),
        };

        for arch in [TargetArch::Avr, TargetArch::Arm] {
            let config = CompilerConfig {
                arch,
                clock_hz: 16_000_000,
                c_includes: vec![],
                c_prelude: vec![],
            };

            let code = generate_c_code(&program, &config);

            assert!(code.contains("__lpc_low_power_sleep_micros(250000ULL);"));
            assert!(code.contains("static inline void __lpc_low_power_sleep_micros"));
        }
    }

    #[test]
    fn test_generate_c_code_keeps_avr_task_only_idle_on_sleep_cpu() {
        let config = CompilerConfig {
            arch: TargetArch::Avr,
            clock_hz: 16_000_000,
            c_includes: vec![],
            c_prelude: vec![],
        };

        let code = generate_c_code(&empty_program(), &config);

        assert!(code.contains("sleep_cpu();"));
        assert!(!code.contains(
            "static inline void enter_idle_sleep_until_interrupt(void) {\n    delay(1);\n}"
        ));
    }
}
