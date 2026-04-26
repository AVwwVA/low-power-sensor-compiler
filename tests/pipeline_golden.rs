use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const AVR_TEST_CONFIG: &str = r#"
arch = "avr"
clock_hz = 16000000
c_includes = ["<Arduino.h>"]
"#;

const ARM_TEST_CONFIG: &str = r#"
arch = "arm"
clock_hz = 16000000
c_includes = ["<Arduino.h>"]
"#;

struct RunResult {
    status: ExitStatus,
    stdout: String,
    stderr: String,
    generated_code: Option<String>,
    out_path: PathBuf,
}

fn compiler_bin_path() -> PathBuf {
    if let Ok(path) = env::var("CARGO_BIN_EXE_low-power-sensor-compiler") {
        return PathBuf::from(path);
    }
    if let Ok(path) = env::var("CARGO_BIN_EXE_low_power_sensor_compiler") {
        return PathBuf::from(path);
    }
    let fallback = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("low-power-sensor-compiler");
    if fallback.exists() {
        return fallback;
    }
    panic!("compiler binary path not found in cargo test environment");
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

fn tests_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name)
}

fn make_temp_dir(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let dir = env::temp_dir().join(format!(
        "lpc_pipeline_golden_{}_{}_{}",
        test_name,
        std::process::id(),
        nanos
    ));
    fs::create_dir_all(&dir).expect("failed to create temp dir");
    dir
}

fn run_fixture_with_config(test_name: &str, fixture_name: &str, config_text: &str) -> RunResult {
    let bin = compiler_bin_path();
    let fixture = fixture_path(fixture_name);
    let temp_dir = make_temp_dir(test_name);
    let config_path = temp_dir.join("compiler_config.toml");
    let out_path = temp_dir.join("out.c");
    fs::write(&config_path, config_text).expect("failed to write temp config");

    let output = Command::new(bin)
        .arg(&fixture)
        .arg("--config")
        .arg(&config_path)
        .arg("--out")
        .arg(&out_path)
        .output()
        .expect("failed to run compiler binary");

    RunResult {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        generated_code: fs::read_to_string(&out_path).ok(),
        out_path,
    }
}

fn run_fixture(test_name: &str, fixture_name: &str) -> RunResult {
    run_fixture_with_config(test_name, fixture_name, AVR_TEST_CONFIG)
}

fn run_source_with_config(test_name: &str, source_name: &str, config_text: &str) -> RunResult {
    let bin = compiler_bin_path();
    let source = tests_path(source_name);
    let temp_dir = make_temp_dir(test_name);
    let config_path = temp_dir.join("compiler_config.toml");
    let out_path = temp_dir.join("out.c");
    fs::write(&config_path, config_text).expect("failed to write temp config");

    let output = Command::new(bin)
        .arg(&source)
        .arg("--config")
        .arg(&config_path)
        .arg("--out")
        .arg(&out_path)
        .output()
        .expect("failed to run compiler binary");

    RunResult {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        generated_code: fs::read_to_string(&out_path).ok(),
        out_path,
    }
}

fn has_location_marker(stderr: &str) -> bool {
    stderr.lines().any(|line| {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("--> ") {
            return false;
        }
        trimmed.matches(':').count() >= 2
    })
}

fn write_text(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("failed to create parent directory");
    }
    fs::write(path, text).expect("failed to write file");
}

fn prepare_c_smoke_include_dir(base_dir: &Path, extra_headers: &[(&str, &str)]) -> PathBuf {
    let include_dir = base_dir.join("include");
    fs::create_dir_all(include_dir.join("avr")).expect("failed to create include dirs");

    write_text(
        &include_dir.join("Arduino.h"),
        r#"#pragma once
#include <stdint.h>
#include <stdbool.h>

#ifndef INPUT
#define INPUT 0
#endif
#ifndef OUTPUT
#define OUTPUT 1
#endif
#ifndef A0
#define A0 0
#endif
#ifndef A1
#define A1 1
#endif
#ifndef D0
#define D0 0
#endif
#ifndef D1
#define D1 1
#endif

typedef struct {
    void (*begin)(int);
    void (*print)(const char*);
    void (*println)(const char*);
} __lpc_serial_t;

static inline void __lpc_serial_begin_impl(int baud) { (void)baud; }
static inline void __lpc_serial_print_impl(const char* msg) { (void)msg; }
static inline void __lpc_serial_println_impl(const char* msg) { (void)msg; }

static const __lpc_serial_t Serial = {
    __lpc_serial_begin_impl,
    __lpc_serial_print_impl,
    __lpc_serial_println_impl
};

static inline int analogRead(int pin) { (void)pin; return 0; }
static inline void analogWrite(int pin, int value) { (void)pin; (void)value; }
static inline void pinMode(int pin, int mode) { (void)pin; (void)mode; }
static inline void delay(unsigned long ms) { (void)ms; }
static inline void delayMicroseconds(unsigned int us) { (void)us; }
static inline uint32_t millis(void) { return 0u; }
static inline void noInterrupts(void) {}
static inline void interrupts(void) {}

extern volatile uint8_t ADCSRA;
extern volatile uint8_t DDRC;
extern volatile uint8_t ADMUX;
extern volatile uint8_t TCCR2A;
extern volatile uint8_t TCCR2B;
extern volatile uint8_t OCR2A;
extern volatile uint8_t TIMSK2;

#ifndef ADEN
#define ADEN 7
#endif
#ifndef ADPS0
#define ADPS0 0
#endif
#ifndef ADPS1
#define ADPS1 1
#endif
#ifndef ADPS2
#define ADPS2 2
#endif
#ifndef WGM21
#define WGM21 1
#endif
#ifndef CS20
#define CS20 0
#endif
#ifndef CS21
#define CS21 1
#endif
#ifndef CS22
#define CS22 2
#endif
#ifndef OCIE2A
#define OCIE2A 1
#endif
"#,
    );

    write_text(
        &include_dir.join("avr/interrupt.h"),
        r#"#pragma once
#include <stdint.h>
#define TIMER2_COMPA_vect __lpc_timer2_compa_vect
#define ISR(vector) void vector(void)
static inline void sei(void) {}
static inline void cli(void) {}
"#,
    );

    write_text(
        &include_dir.join("avr/sleep.h"),
        r#"#pragma once
#define SLEEP_MODE_IDLE 0
static inline void set_sleep_mode(int mode) { (void)mode; }
static inline void sleep_enable(void) {}
static inline void sleep_disable(void) {}
static inline void sleep_cpu(void) {}
static inline void sleep_bod_disable(void) {}
"#,
    );

    write_text(&include_dir.join("avr/power.h"), "#pragma once\n");
    write_text(
        &include_dir.join("cmsis_gcc.h"),
        r#"#pragma once
#define __DSB() do {} while (0)
#define __WFI() do {} while (0)
#define __ISB() do {} while (0)
"#,
    );

    for (header, contents) in extra_headers {
        write_text(&include_dir.join(header), contents);
    }

    include_dir
}

fn compile_c11_with_tool(tool: &str, source_file: &Path, include_dir: &Path) -> Output {
    Command::new(tool)
        .arg("-std=c11")
        .arg("-fsyntax-only")
        .arg("-x")
        .arg("c")
        .arg("-I")
        .arg(include_dir)
        .arg(source_file)
        .output()
        .unwrap_or_else(|err| panic!("failed to execute '{}': {}", tool, err))
}

fn is_tool_available(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

fn run_optional_cross_c11_smoke(tool: &str, source_file: &Path, include_dir: &Path) {
    if !is_tool_available(tool) {
        eprintln!("skipping optional {} C11 smoke (tool not found)", tool);
        return;
    }

    let output = compile_c11_with_tool(tool, source_file, include_dir);
    assert!(
        output.status.success(),
        "optional {} C11 smoke failed:\n{}",
        tool,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_host_and_optional_c11_smoke(
    test_name: &str,
    source_file: &Path,
    extra_headers: &[(&str, &str)],
) -> Output {
    let smoke_dir = make_temp_dir(&format!("{}_smoke", test_name));
    let include_dir = prepare_c_smoke_include_dir(&smoke_dir, extra_headers);
    let host_output = compile_c11_with_tool("gcc", source_file, &include_dir);
    run_optional_cross_c11_smoke("avr-gcc", source_file, &include_dir);
    run_optional_cross_c11_smoke("arm-none-eabi-gcc", source_file, &include_dir);
    host_output
}

fn prepare_runtime_include_dir(base_dir: &Path) -> PathBuf {
    let include_dir = prepare_c_smoke_include_dir(base_dir, &[]);
    write_text(
        &include_dir.join("Arduino.h"),
        r#"#pragma once
#include <stdint.h>
#include <stdbool.h>
#include <stdio.h>

#ifndef INPUT
#define INPUT 0
#endif
#ifndef OUTPUT
#define OUTPUT 1
#endif
#ifndef A0
#define A0 0
#endif
#ifndef D0
#define D0 0
#endif

typedef struct {
    void (*begin)(int);
    void (*print)(const char*);
    void (*println)(const char*);
} __lpc_serial_t;

static inline void __lpc_serial_begin_impl(int baud) { (void)baud; }
static inline void __lpc_serial_print_impl(const char* msg) { fputs(msg == NULL ? "" : msg, stdout); }
static inline void __lpc_serial_println_impl(const char* msg) { puts(msg == NULL ? "" : msg); }

static const __lpc_serial_t Serial = {
    __lpc_serial_begin_impl,
    __lpc_serial_print_impl,
    __lpc_serial_println_impl
};

static inline int analogRead(int pin) { (void)pin; return 0; }
static inline void analogWrite(int pin, int value) { (void)pin; (void)value; }
static inline void pinMode(int pin, int mode) { (void)pin; (void)mode; }
static inline void delay(unsigned long ms) { (void)ms; }
static inline void delayMicroseconds(unsigned int us) { (void)us; }
static inline uint32_t millis(void) { return 0u; }
static inline void noInterrupts(void) {}
static inline void interrupts(void) {}
"#,
    );
    include_dir
}

fn compile_and_run_runtime_task(
    test_name: &str,
    generated_code: &Path,
    task_symbol: &str,
) -> Output {
    let runtime_dir = make_temp_dir(&format!("{}_runtime", test_name));
    let include_dir = prepare_runtime_include_dir(&runtime_dir);
    let runner_path = runtime_dir.join("runner.c");
    let binary_path = runtime_dir.join("runtime.out");
    write_text(
        &runner_path,
        &format!(
            "void {task_symbol}(void);\nint main(void) {{\n    {task_symbol}();\n    return 0;\n}}\n"
        ),
    );

    let compile = Command::new("gcc")
        .arg("-std=c11")
        .arg("-I")
        .arg(&include_dir)
        .arg(generated_code)
        .arg(&runner_path)
        .arg("-o")
        .arg(&binary_path)
        .output()
        .expect("failed to compile runtime harness");
    assert!(
        compile.status.success(),
        "runtime gcc compile failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    Command::new(&binary_path)
        .output()
        .expect("failed to run runtime harness")
}

#[test]
fn golden_typed_temp_read_success() {
    let run = run_fixture(
        "typed_temp_read_success",
        "golden_typed_temp_read_success.lpc",
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);
    let code = run
        .generated_code
        .expect("expected generated output for success fixture");
    assert!(
        code.contains("float t = convert_temp(analogRead(temp));"),
        "generated code did not contain expected typed temp read:\n{}",
        code
    );
}

#[test]
fn golden_typed_time_read_success() {
    let run = run_fixture(
        "typed_time_read_success",
        "golden_typed_time_read_success.lpc",
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);
    let code = run
        .generated_code
        .expect("expected generated output for success fixture");
    assert!(
        code.contains("int t = convert_tick(analogRead(tick_sensor));"),
        "generated code did not contain expected typed time read:\n{}",
        code
    );
}

#[test]
fn golden_time_cycle_error() {
    let run = run_fixture("time_cycle_error", "golden_time_cycle_error.lpc");
    assert!(
        !run.status.success(),
        "expected compiler failure, stdout:\n{}",
        run.stdout
    );
    assert!(run.stderr.contains("Invalid 'every' interval"));
    assert!(run.stderr.to_lowercase().contains("invalid value"));
    assert!(
        has_location_marker(&run.stderr),
        "expected diagnostic location marker in stderr:\n{}",
        run.stderr
    );
}

#[test]
fn golden_time_overflow_every_error() {
    let run = run_fixture(
        "time_overflow_every_error",
        "golden_time_overflow_every_error.lpc",
    );
    assert!(
        !run.status.success(),
        "expected compiler failure, stdout:\n{}",
        run.stdout
    );
    assert!(run.stderr.contains("Invalid 'every' interval"));
    assert!(run.stderr.contains("overflows"));
    assert!(
        has_location_marker(&run.stderr),
        "expected diagnostic location marker in stderr:\n{}",
        run.stderr
    );
}

#[test]
fn golden_time_overflow_sleep_error() {
    let run = run_fixture(
        "time_overflow_sleep_error",
        "golden_time_overflow_sleep_error.lpc",
    );
    assert!(
        !run.status.success(),
        "expected compiler failure, stdout:\n{}",
        run.stdout
    );
    assert!(run.stderr.contains("Invalid sleep duration"));
    assert!(run.stderr.contains("overflows"));
    assert!(
        has_location_marker(&run.stderr),
        "expected diagnostic location marker in stderr:\n{}",
        run.stderr
    );
}

#[test]
fn golden_string_escaping_success() {
    let run = run_fixture(
        "string_escaping_success",
        "golden_string_escaping_success.lpc",
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);
    let code = run
        .generated_code
        .expect("expected generated output for success fixture");
    assert!(
        code.contains("Serial.println(\"line1\\n\\\"quote\\\"\\\\path\\tend\\r\");"),
        "generated code did not contain expected escaped string literal:\n{}",
        code
    );
}

#[test]
fn golden_explicit_sleep_uses_low_power_helper() {
    for (label, config) in [("avr", AVR_TEST_CONFIG), ("arm", ARM_TEST_CONFIG)] {
        let run = run_fixture_with_config(
            &format!("explicit_sleep_low_power_{}", label),
            "golden_explicit_sleep_success.lpc",
            config,
        );
        assert!(run.status.success(), "stderr:\n{}", run.stderr);
        let code = run
            .generated_code
            .expect("expected generated output for explicit sleep fixture");
        assert!(
            code.contains("__lpc_low_power_sleep_micros(1000000ULL);"),
            "generated code did not contain expected explicit sleep helper call:\n{}",
            code
        );
        assert!(
            code.contains("static inline void __lpc_low_power_sleep_micros"),
            "generated code did not define the low-power sleep helper:\n{}",
            code
        );

        let smoke = run_host_and_optional_c11_smoke(
            &format!("explicit_sleep_low_power_{}", label),
            &run.out_path,
            &[],
        );
        assert!(
            smoke.status.success(),
            "host gcc C11 smoke failed:\n{}",
            String::from_utf8_lossy(&smoke.stderr)
        );
    }
}

#[test]
fn golden_deterministic_nested_locals_success() {
    let run = run_fixture(
        "deterministic_nested_locals_success",
        "golden_deterministic_nested_locals_success.lpc",
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);
    let code = run
        .generated_code
        .expect("expected generated output for success fixture");
    let alpha_pos = code
        .find("int alpha = 0;")
        .expect("expected alpha predeclaration in generated output");
    let zeta_pos = code
        .find("int zeta = 0;")
        .expect("expected zeta predeclaration in generated output");
    assert!(
        alpha_pos < zeta_pos,
        "expected deterministic alphabetical declaration order, got:\n{}",
        code
    );
}

#[test]
fn golden_task_setup_only_arm_codegen_and_compile_smoke() {
    let run = run_fixture_with_config(
        "task_setup_only_arm",
        "golden_task_setup_only_success.lpc",
        ARM_TEST_CONFIG,
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);
    let code = run
        .generated_code
        .expect("expected generated output for success fixture");

    let serial_pos = code
        .find("Serial.begin(9600);")
        .expect("expected Serial.begin in setup");
    let pin_pos = code
        .find("pinMode(A0, INPUT);")
        .expect("expected sensor pinMode in setup");
    let task_stmt_pos = code
        .find("int once = analogRead(temp);")
        .expect("expected one-shot read statement in setup");
    assert!(serial_pos < pin_pos && pin_pos < task_stmt_pos);
    assert!(
        !code.contains("task_ready_mask"),
        "task-only output should not include periodic scheduler globals:\n{}",
        code
    );

    let smoke = run_host_and_optional_c11_smoke("task_setup_only_arm", &run.out_path, &[]);
    assert!(
        smoke.status.success(),
        "host gcc C11 smoke failed:\n{}",
        String::from_utf8_lossy(&smoke.stderr)
    );
}

#[test]
fn golden_task_setup_only_avr_codegen_and_compile_smoke() {
    let run = run_fixture_with_config(
        "task_setup_only_avr",
        "golden_task_setup_only_success.lpc",
        AVR_TEST_CONFIG,
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);
    let code = run
        .generated_code
        .expect("expected generated output for success fixture");

    let serial_pos = code
        .find("Serial.begin(9600);")
        .expect("expected Serial.begin in setup");
    let pin_pos = code
        .find("pinMode(A0, INPUT);")
        .expect("expected sensor pinMode in setup");
    let task_stmt_pos = code
        .find("int once = analogRead(temp);")
        .expect("expected one-shot read statement in setup");
    assert!(serial_pos < pin_pos && pin_pos < task_stmt_pos);
    assert!(
        !code.contains("task_ready_mask"),
        "task-only output should not include periodic scheduler globals:\n{}",
        code
    );

    let smoke = run_host_and_optional_c11_smoke("task_setup_only_avr", &run.out_path, &[]);
    assert!(
        smoke.status.success(),
        "host gcc C11 smoke failed:\n{}",
        String::from_utf8_lossy(&smoke.stderr)
    );
}

#[test]
fn golden_task_and_every_codegen_split_success() {
    let run = run_fixture_with_config(
        "task_and_every_split",
        "golden_task_and_every_success.lpc",
        ARM_TEST_CONFIG,
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);
    let code = run
        .generated_code
        .expect("expected generated output for success fixture");

    let setup_boot_pos = code
        .find("int boot = 1;")
        .expect("expected one-shot task statement in setup");
    let setup_start = code
        .find("void setup(void)")
        .expect("expected setup function");
    let setup_end = code
        .find("void loop(void)")
        .expect("expected loop function");
    assert!(setup_start < setup_boot_pos && setup_boot_pos < setup_end);

    assert!(
        code.contains("void task_0(void)"),
        "expected periodic task function for every block:\n{}",
        code
    );
    assert!(
        code.contains("int t = analogRead(temp);"),
        "expected periodic read statement in task function:\n{}",
        code
    );
    assert!(
        code.contains("task_period_ticks"),
        "expected periodic scheduler state for every block:\n{}",
        code
    );
}

#[test]
fn golden_arm_c11_compile_smoke_success() {
    let run = run_fixture_with_config(
        "arm_c11_compile_smoke",
        "golden_typed_temp_read_success.lpc",
        ARM_TEST_CONFIG,
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);
    let smoke = run_host_and_optional_c11_smoke(
        "arm_c11_compile_smoke",
        &run.out_path,
        &[("config_smoke.h", "#pragma once\n")],
    );
    assert!(
        smoke.status.success(),
        "host gcc C11 smoke failed:\n{}",
        String::from_utf8_lossy(&smoke.stderr)
    );
}

#[test]
fn golden_avr_c11_compile_smoke_success() {
    let run = run_fixture_with_config(
        "avr_c11_compile_smoke",
        "golden_typed_temp_read_success.lpc",
        AVR_TEST_CONFIG,
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);
    let smoke = run_host_and_optional_c11_smoke(
        "avr_c11_compile_smoke",
        &run.out_path,
        &[("config_smoke.h", "#pragma once\n")],
    );
    assert!(
        smoke.status.success(),
        "host gcc C11 smoke failed:\n{}",
        String::from_utf8_lossy(&smoke.stderr)
    );
}

#[test]
fn golden_avr_scheduler_validation_failure_reports_codegen_error_with_span() {
    let invalid_avr_config = r#"
arch = "avr"
clock_hz = 14745600
c_includes = ["<Arduino.h>"]
"#;
    let run = run_fixture_with_config(
        "avr_scheduler_validation_failure",
        "golden_typed_temp_read_success.lpc",
        invalid_avr_config,
    );
    assert!(
        !run.status.success(),
        "expected backend validation failure, stdout:\n{}",
        run.stdout
    );
    assert!(run.stderr.contains("codegen error"));
    assert!(run
        .stderr
        .contains("cannot be represented exactly with available Timer2 prescalers"));
    assert!(
        has_location_marker(&run.stderr),
        "expected location marker in stderr:\n{}",
        run.stderr
    );
}

#[test]
fn golden_config_passthrough_valid_is_preserved_and_compiles() {
    let config = r##"
arch = "arm"
clock_hz = 16000000
c_includes = ["<Arduino.h>", "<math.h>"]
c_prelude = [
    "#define LPC_CFG_OK 1",
    "int lpc_cfg_marker = 7;"
]
"##;
    let run = run_fixture_with_config(
        "config_passthrough_valid",
        "golden_typed_temp_read_success.lpc",
        config,
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);
    let code = run
        .generated_code
        .expect("expected generated output for success fixture");
    assert!(code.contains("#include <math.h>"));
    assert!(code.contains("#define LPC_CFG_OK 1"));
    assert!(code.contains("int lpc_cfg_marker = 7;"));

    let smoke = run_host_and_optional_c11_smoke("config_passthrough_valid", &run.out_path, &[]);
    assert!(
        smoke.status.success(),
        "host gcc C11 smoke failed:\n{}",
        String::from_utf8_lossy(&smoke.stderr)
    );
}

#[test]
fn golden_config_passthrough_malformed_fails_c11_smoke() {
    let config = r##"
arch = "arm"
clock_hz = 16000000
c_includes = ["<Arduino.h>"]
c_prelude = [
    "int lpc_cfg_broken = ;"
]
"##;
    let run = run_fixture_with_config(
        "config_passthrough_malformed",
        "golden_typed_temp_read_success.lpc",
        config,
    );
    assert!(
        run.status.success(),
        "compiler should pass raw passthrough text through to output, stderr:\n{}",
        run.stderr
    );

    let smoke = run_host_and_optional_c11_smoke("config_passthrough_malformed", &run.out_path, &[]);
    assert!(
        !smoke.status.success(),
        "expected C11 smoke failure for malformed passthrough prelude"
    );
    let smoke_stderr = String::from_utf8_lossy(&smoke.stderr);
    assert!(
        smoke_stderr.contains("lpc_cfg_broken"),
        "expected deterministic malformed token in compile stderr:\n{}",
        smoke_stderr
    );
}

fn assert_runtime_cast_helper_paths(code: &str) {
    for helper_call in [
        "__lpc_to_int_from_string(",
        "__lpc_to_float_from_string(",
        "__lpc_to_bool_from_string(",
        "__lpc_format_int_to_buffer(",
        "__lpc_format_float_to_buffer(",
        "__lpc_format_bool_to_buffer(",
        "__lpc_append_cstr(",
    ] {
        assert!(
            code.contains(helper_call),
            "expected generated code to contain helper call '{}', code:\n{}",
            helper_call,
            code
        );
    }

    for marker in ["CAST_BEGIN", "CAST_END", "CAST|concat_mix|"] {
        assert!(
            code.contains(marker),
            "expected generated code to contain marker '{}', code:\n{}",
            marker,
            code
        );
    }
}

#[test]
fn runtime_cast_matrix_arm_compile_smoke_and_helper_paths() {
    let run = run_source_with_config(
        "runtime_cast_matrix_arm",
        "runtime_cast_matrix.lpc",
        ARM_TEST_CONFIG,
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);
    let code = run
        .generated_code
        .expect("expected generated output for runtime cast matrix");
    assert_runtime_cast_helper_paths(&code);

    let smoke = run_host_and_optional_c11_smoke(
        "runtime_cast_matrix_arm",
        &run.out_path,
        &[("config_smoke.h", "#pragma once\n")],
    );
    assert!(
        smoke.status.success(),
        "host gcc C11 smoke failed:\n{}",
        String::from_utf8_lossy(&smoke.stderr)
    );
}

#[test]
fn runtime_cast_matrix_avr_compile_smoke_and_helper_paths() {
    let run = run_source_with_config(
        "runtime_cast_matrix_avr",
        "runtime_cast_matrix.lpc",
        AVR_TEST_CONFIG,
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);
    let code = run
        .generated_code
        .expect("expected generated output for runtime cast matrix");
    assert_runtime_cast_helper_paths(&code);

    let smoke = run_host_and_optional_c11_smoke(
        "runtime_cast_matrix_avr",
        &run.out_path,
        &[("config_smoke.h", "#pragma once\n")],
    );
    assert!(
        smoke.status.success(),
        "host gcc C11 smoke failed:\n{}",
        String::from_utf8_lossy(&smoke.stderr)
    );
}

#[test]
fn runtime_cast_matrix_arm_runtime_concat_chain_preserves_order() {
    let run = run_source_with_config(
        "runtime_cast_matrix_arm_runtime",
        "runtime_cast_matrix.lpc",
        ARM_TEST_CONFIG,
    );
    assert!(run.status.success(), "stderr:\n{}", run.stderr);

    let output = compile_and_run_runtime_task(
        "runtime_cast_matrix_arm_runtime",
        &run.out_path,
        "task_0",
    );
    assert!(
        output.status.success(),
        "runtime harness failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("CAST|concat_mix|A=7,B=2.5,C=true"),
        "expected concat output order to be preserved, stdout:\n{}",
        stdout
    );
    assert!(
        !stdout.contains('?'),
        "unexpected placeholder character in runtime output:\n{}",
        stdout
    );
}
