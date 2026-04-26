pub mod arm;
pub mod avr;
pub mod utils;

use crate::config::{CompilerConfig, TargetArch};
use crate::diagnostics::SourceSpan;
use crate::task_ir::IrProgram;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodegenError {
    UnsupportedIr {
        message: String,
        source_span: Option<SourceSpan>,
    },
    InvalidSchedulerConfig {
        message: String,
        source_span: Option<SourceSpan>,
    },
}

impl CodegenError {
    pub fn unsupported_ir(
        message: impl Into<String>,
        source_span: Option<SourceSpan>,
    ) -> Self {
        Self::UnsupportedIr {
            message: message.into(),
            source_span,
        }
    }

    pub fn invalid_scheduler_config(
        message: impl Into<String>,
        source_span: Option<SourceSpan>,
    ) -> Self {
        Self::InvalidSchedulerConfig {
            message: message.into(),
            source_span,
        }
    }

    pub fn source_span(&self) -> Option<SourceSpan> {
        match self {
            Self::UnsupportedIr { source_span, .. }
            | Self::InvalidSchedulerConfig { source_span, .. } => *source_span,
        }
    }
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedIr { message, .. }
            | Self::InvalidSchedulerConfig { message, .. } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for CodegenError {}

pub type CodegenResult<T> = Result<T, CodegenError>;

pub(crate) fn first_periodic_task_span(program: &IrProgram) -> Option<SourceSpan> {
    program.tasks.first().and_then(|task| task.source_span)
}

pub fn generate_c_code(program: &IrProgram, config: &CompilerConfig) -> CodegenResult<String> {
    match config.arch {
        TargetArch::Avr => avr::generate(program, config),
        TargetArch::Arm => arm::generate(program, config),
    }
}

#[cfg(test)]
mod tests {
    use super::generate_c_code;
    use crate::config::{CompilerConfig, TargetArch};
    use crate::task_ir::{
        EnergyPolicy, IrProgram, IrStmt, IrTask, PowerState, SchedulerConfig, SleepFallback,
        TaskTrigger, WakeSource,
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

    fn periodic_program() -> IrProgram {
        IrProgram {
            definitions: vec![],
            tasks: vec![IrTask {
                name: None,
                trigger: TaskTrigger::Periodic {
                    period_ticks: 1,
                    phase_ticks: 0,
                },
                body: vec![],
                source_span: None,
            }],
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

            let code = generate_c_code(&empty_program(), &config).expect("codegen should succeed");

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

            let code = generate_c_code(&program, &config).expect("codegen should succeed");

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

        let code = generate_c_code(&empty_program(), &config).expect("codegen should succeed");

        assert!(code.contains("sleep_cpu();"));
        assert!(!code.contains(
            "static inline void enter_idle_sleep_until_interrupt(void) {\n    delay(1);\n}"
        ));
    }

    #[test]
    fn test_arm_scheduler_validation_rejects_sub_millisecond_tick() {
        let mut program = periodic_program();
        program.scheduler.tick_micros = 500;
        let config = CompilerConfig {
            arch: TargetArch::Arm,
            clock_hz: 16_000_000,
            c_includes: vec![],
            c_prelude: vec![],
        };

        let err = generate_c_code(&program, &config).expect_err("ARM codegen should fail");
        assert!(err.to_string().contains("at least 1000 microseconds"));
    }

    #[test]
    fn test_arm_scheduler_validation_rejects_non_millisecond_multiple() {
        let mut program = periodic_program();
        program.scheduler.tick_micros = 1500;
        let config = CompilerConfig {
            arch: TargetArch::Arm,
            clock_hz: 16_000_000,
            c_includes: vec![],
            c_prelude: vec![],
        };

        let err = generate_c_code(&program, &config).expect_err("ARM codegen should fail");
        assert!(err.to_string().contains("exact multiple of 1000"));
    }

    #[test]
    fn test_arm_scheduler_validation_accepts_exact_millisecond_multiple() {
        let mut program = periodic_program();
        program.scheduler.tick_micros = 2000;
        let config = CompilerConfig {
            arch: TargetArch::Arm,
            clock_hz: 16_000_000,
            c_includes: vec![],
            c_prelude: vec![],
        };

        let code = generate_c_code(&program, &config).expect("ARM codegen should succeed");
        assert!(code.contains("static const uint16_t SCHED_TICK_MS = 2;"));
    }

    #[test]
    fn test_avr_scheduler_validation_rejects_inexact_division() {
        let program = periodic_program();
        let config = CompilerConfig {
            arch: TargetArch::Avr,
            clock_hz: 14_745_600,
            c_includes: vec![],
            c_prelude: vec![],
        };

        let err = generate_c_code(&program, &config).expect_err("AVR codegen should fail");
        assert!(err
            .to_string()
            .contains("cannot be represented exactly with available Timer2 prescalers"));
    }

    #[test]
    fn test_avr_scheduler_validation_rejects_out_of_range_compare() {
        let program = periodic_program();
        let config = CompilerConfig {
            arch: TargetArch::Avr,
            clock_hz: 263_168_000,
            c_includes: vec![],
            c_prelude: vec![],
        };

        let err = generate_c_code(&program, &config).expect_err("AVR codegen should fail");
        assert!(err.to_string().contains("outside 0..=255"));
    }

    #[test]
    fn test_avr_scheduler_validation_accepts_exact_fit() {
        let program = periodic_program();
        let config = CompilerConfig {
            arch: TargetArch::Avr,
            clock_hz: 16_000_000,
            c_includes: vec![],
            c_prelude: vec![],
        };

        let code = generate_c_code(&program, &config).expect("AVR codegen should succeed");
        assert!(code.contains("TCCR2B = (1 << CS22);"));
        assert!(code.contains("OCR2A = 249;"));
    }
}
