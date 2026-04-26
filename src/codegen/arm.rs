use crate::codegen::utils::{
    format_autocast_helpers, format_includes, format_prelude, format_program_metadata,
    generate_function, generate_setup_body, generate_task, task_symbol,
};
use crate::codegen::{first_periodic_task_span, CodegenError, CodegenResult};
use crate::config::CompilerConfig;
use crate::task_ir::{IrDefinition, IrProgram, TaskTrigger};

fn validate_scheduler_tick(program: &IrProgram) -> CodegenResult<u32> {
    let tick_micros = program.scheduler.tick_micros;
    let source_span = first_periodic_task_span(program);

    if tick_micros < 1000 {
        return Err(CodegenError::invalid_scheduler_config(
            format!(
                "ARM scheduler tick must be at least 1000 microseconds, got {}",
                tick_micros
            ),
            source_span,
        ));
    }
    if !tick_micros.is_multiple_of(1000) {
        return Err(CodegenError::invalid_scheduler_config(
            format!(
                "ARM scheduler tick must be an exact multiple of 1000 microseconds, got {}",
                tick_micros
            ),
            source_span,
        ));
    }

    let sched_tick_ms = tick_micros / 1000;
    if sched_tick_ms == 0 {
        return Err(CodegenError::invalid_scheduler_config(
            "ARM scheduler tick cannot be reduced to 0 milliseconds".to_string(),
            source_span,
        ));
    }

    Ok(sched_tick_ms)
}

pub fn generate(program: &IrProgram, config: &CompilerConfig) -> CodegenResult<String> {
    let mut code = String::new();

    code.push_str(&format_includes(
        config,
        &[
            "<Arduino.h>",
            "<stdint.h>",
            "<stdbool.h>",
            "<stddef.h>",
            "<stdlib.h>",
            "<stdio.h>",
        ],
    ));

    code.push_str(
        r#"
#if defined(__arm__) || defined(__ARM_ARCH)
#include <cmsis_gcc.h>
#endif

"#,
    );
    code.push_str(&format_prelude(config));
    code.push_str(&format_program_metadata(program));
    code.push_str(&format_autocast_helpers());
    code.push_str(
        r#"
static inline void enter_idle_sleep_until_interrupt(void) {
#if defined(__arm__) || defined(__ARM_ARCH)
    __DSB();
    __WFI();
    __ISB();
#else
    delay(1);
#endif
}

static inline void __lpc_low_power_sleep_micros(uint64_t duration_us) {
    uint64_t remaining_ms = duration_us / 1000u;
    uint16_t remainder_us = (uint16_t)(duration_us % 1000u);

    while (remaining_ms > 0u) {
        uint32_t chunk_ms = remaining_ms > 60000u ? 60000u : (uint32_t)remaining_ms;
        uint32_t start_ms = millis();
        while ((uint32_t)(millis() - start_ms) < chunk_ms) {
            enter_idle_sleep_until_interrupt();
        }
        remaining_ms -= (uint64_t)chunk_ms;
    }

    if (remainder_us > 0u) {
        delayMicroseconds((unsigned int)remainder_us);
    }
}

"#,
    );

    let task_count = program.tasks.len();
    code.push_str(&format!(
        "static const uint8_t TASK_COUNT = {};\n",
        task_count
    ));

    if task_count > 0 {
        let sched_tick_ms = validate_scheduler_tick(program)?;
        code.push_str(&format!(
            "static const uint16_t SCHED_TICK_MS = {};\n\n",
            sched_tick_ms
        ));

        code.push_str("volatile uint8_t task_ready_mask = 0;\n");
        let mut periods = Vec::new();
        for task in &program.tasks {
            match &task.trigger {
                TaskTrigger::Periodic { period_ticks, .. } => {
                    periods.push(period_ticks.to_string());
                }
            }
        }
        code.push_str(&format!(
            "volatile uint16_t task_period_ticks[TASK_COUNT] = {{{}}};\n",
            periods.join(", ")
        ));
        let zeros = vec!["0"; task_count].join(", ");
        code.push_str(&format!(
            "volatile uint16_t task_elapsed_ticks[TASK_COUNT] = {{{}}};\n\n",
            zeros
        ));
    } else {
        code.push('\n');
    }

    for def in &program.definitions {
        if let IrDefinition::Sensor(s) = def {
            code.push_str(&format!("volatile int16_t {} = 0;\n", s.name));
        }
    }
    code.push('\n');

    for func in &program.functions {
        code.push_str(&generate_function(func)?);
    }

    for (i, task) in program.tasks.iter().enumerate() {
        code.push_str(&generate_task(i, task)?);
    }

    if task_count > 0 {
        code.push_str(
            r#"
static inline void scheduler_on_tick(void) {
    uint8_t i;
    for (i = 0; i < TASK_COUNT; ++i) {
        uint16_t period = task_period_ticks[i];
        uint16_t next;
        if (period == 0) {
            continue;
        }

        next = (uint16_t)(task_elapsed_ticks[i] + 1u);
        if (next >= period) {
            task_ready_mask |= (uint8_t)(1u << i);
            next = 0u;
        }
        task_elapsed_ticks[i] = next;
    }
}

static inline void scheduler_process_elapsed_ticks(uint32_t elapsed_ms) {
    while (elapsed_ms >= SCHED_TICK_MS) {
        noInterrupts();
        scheduler_on_tick();
        interrupts();
        elapsed_ms -= SCHED_TICK_MS;
    }
}
"#,
        );
    }

    code.push_str("void setup(void) {\n");
    code.push_str("    Serial.begin(9600);\n");

    for def in &program.definitions {
        if let IrDefinition::Sensor(s) = def {
            code.push_str(&format!("    pinMode({}, INPUT);\n", s.pin));
        }
        if let IrDefinition::Output(o) = def {
            code.push_str(&format!("    pinMode({}, OUTPUT);\n", o.pin));
        }
    }

    if !program.setup_body.is_empty() {
        code.push('\n');
        code.push_str(&generate_setup_body(&program.setup_body, "    ")?);
    }

    code.push_str("}\n\n");

    if task_count == 0 {
        code.push_str(
            r#"void loop(void) {
    enter_idle_sleep_until_interrupt();
}
"#,
        );
        return Ok(code);
    }

    code.push_str(
        r#"void loop(void) {
    static uint32_t last_tick_ms = 0;
    uint32_t now_ms;
    uint32_t elapsed_ms;
    uint8_t ready_snapshot;
    uint8_t i;

    now_ms = millis();
    elapsed_ms = (uint32_t)(now_ms - last_tick_ms);

    if (elapsed_ms >= SCHED_TICK_MS) {
        uint32_t consumed = elapsed_ms - (elapsed_ms % SCHED_TICK_MS);
        last_tick_ms += consumed;
        scheduler_process_elapsed_ticks(consumed);
    }

    noInterrupts();
    ready_snapshot = task_ready_mask;
    task_ready_mask = 0;
    interrupts();

    for (i = 0; i < TASK_COUNT; ++i) {
        if ((ready_snapshot & (uint8_t)(1u << i)) == 0u) {
            continue;
        }

        switch (i) {
"#,
    );

    for (i, task) in program.tasks.iter().enumerate() {
        code.push_str(&format!(
            "            case {}:\n                {}();\n                break;\n",
            i,
            task_symbol(task, i)
        ));
    }

    code.push_str(
        r#"            default:
                break;
        }
    }

    if (ready_snapshot == 0u) {
        enter_idle_sleep_until_interrupt();
    }
}
"#,
    );

    Ok(code)
}
