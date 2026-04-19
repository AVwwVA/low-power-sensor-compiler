use crate::codegen::utils::{
    format_autocast_helpers, format_includes, format_prelude, format_program_metadata,
    generate_function, generate_setup_body, generate_task, task_symbol,
};
use crate::config::CompilerConfig;
use crate::task_ir::{IrDefinition, IrProgram, TaskTrigger};

pub fn generate(program: &IrProgram, config: &CompilerConfig) -> String {
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
            "<avr/interrupt.h>",
            "<avr/sleep.h>",
            "<avr/power.h>",
        ],
    ));
    code.push_str(&format_prelude(config));
    code.push_str(&format_program_metadata(program));
    code.push_str(&format_autocast_helpers());
    code.push_str(
        r#"
static inline void enter_idle_sleep_until_interrupt(void) {
    set_sleep_mode(SLEEP_MODE_IDLE);

    const uint8_t adcsra_saved = ADCSRA;
    ADCSRA &= (uint8_t)(~(1u << ADEN));

    sleep_enable();
#if defined(BODS) && defined(BODSE)
    sleep_bod_disable();
#endif
    sei();
    sleep_cpu();
    sleep_disable();

    ADCSRA = adcsra_saved;
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
        "static const uint8_t TASK_COUNT = {};\n\n",
        task_count
    ));

    if task_count > 0 {
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
    }

    for def in &program.definitions {
        if let IrDefinition::Sensor(s) = def {
            code.push_str(&format!("volatile int16_t {} = 0;\n", s.name));
        }
    }
    code.push('\n');

    for func in &program.functions {
        code.push_str(&generate_function(func));
    }

    for (i, task) in program.tasks.iter().enumerate() {
        code.push_str(&generate_task(i, task));
    }

    if task_count > 0 {
        code.push_str(
            r#"
ISR(TIMER2_COMPA_vect) {
    for (uint8_t task_idx = 0; task_idx < TASK_COUNT; ++task_idx) {
        const uint16_t period = task_period_ticks[task_idx];
        if (period == 0) {
            continue;
        }

        uint16_t next = task_elapsed_ticks[task_idx] + 1;
        if (next >= period) {
            task_ready_mask |= (1u << task_idx);
            next = 0;
        }
        task_elapsed_ticks[task_idx] = next;
    }
}
"#,
        );
    }

    code.push_str("void setup() {\n");
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
        code.push_str(&generate_setup_body(&program.setup_body, "    "));
    }

    if task_count > 0 {
        let tick_micros = program.scheduler.tick_micros as u64;
        let clock_hz = config.clock_hz as u64;
        let prescaler = 1024;
        let ocr2a = (tick_micros * clock_hz) / (prescaler * 1_000_000) - 1;

        code.push_str(&format!(
            r#"
    DDRC &= (uint8_t)(~0x0F);

    ADMUX = 0x40;
    ADCSRA = (uint8_t)((1 << ADEN) | (1 << ADPS2) | (1 << ADPS1) | (1 << ADPS0));

    TCCR2A = (1 << WGM21);
    TCCR2B = (1 << CS22) | (1 << CS21) | (1 << CS20);
    OCR2A = {};
    TIMSK2 = (1 << OCIE2A);

    sei();
"#,
            ocr2a
        ));
    }

    code.push_str("}\n\n");

    if task_count == 0 {
        code.push_str(
            r#"void loop() {
    enter_idle_sleep_until_interrupt();
}
"#,
        );
        return code;
    }

    code.push_str(
        r#"void loop() {
    uint8_t ready_snapshot;

    cli();
    ready_snapshot = task_ready_mask;
    task_ready_mask = 0;
    sei();

    for (uint8_t i = 0; i < TASK_COUNT; ++i) {
        if ((ready_snapshot & (1u << i)) == 0) {
            continue;
        }

        switch (i) {
"#,
    );

    for (i, task) in program.tasks.iter().enumerate() {
        code.push_str(&format!(
            "            case {}: {}(); break;\n",
            i,
            task_symbol(task, i)
        ));
    }

    code.push_str(
        r#"            default: break;
        }
    }

    if (ready_snapshot == 0) {
        enter_idle_sleep_until_interrupt();
    }
}
"#,
    );

    code
}
