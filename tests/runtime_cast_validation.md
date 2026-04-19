# Runtime Cast Validation (AVR + ARM)

This protocol validates runtime autocasting behavior on one AVR board and one ARM board using `tests/runtime_cast_matrix.lpc`.

## Worksheet (fill at run time)

- Operator:
- Date/Time:
- Board model:
- MCU family:
- Port:
- Baud:
- Firmware timestamp/hash:
- Target config used:

## Build and Flash Procedure

1. Generate output code from `tests/runtime_cast_matrix.lpc` for your board target.
2. Build and upload to the selected board using your normal Arduino workflow.
3. Open Serial Monitor at the selected baud (default `9600` unless your board profile differs).
4. Capture the first full block from `CAST_BEGIN` to `CAST_END`.
5. Copy the captured block into the worksheet section below.

## AVR Run Steps

1. Select the AVR board and correct serial port in Arduino IDE/CLI.
2. Use AVR compiler configuration.
3. Upload firmware.
4. Open Serial Monitor and capture one full `CAST_BEGIN..CAST_END` block.
5. Fill checklist and pass/fail table.

## ARM Run Steps

1. Select the ARM board and correct serial port in Arduino IDE/CLI.
2. Use ARM compiler configuration.
3. Upload firmware.
4. Open Serial Monitor and capture one full `CAST_BEGIN..CAST_END` block.
5. Fill checklist and pass/fail table.

## Capture Block

Paste first full cycle here:

```text
CAST_BEGIN
...
CAST_END
```

## Pass/Fail Checklist

- `CAST_BEGIN` appears before any `CAST|...|...` line.
- `CAST_END` appears after all cast lines.
- No line value contains `?`.
- All required `case_id` lines are present exactly once in the captured cycle.

Required `case_id` values:

- `s2i`
- `s2f`
- `s2b_empty`
- `s2b_text`
- `f2i`
- `i2f`
- `i2s`
- `f2s`
- `b2s_true`
- `b2s_false`
- `concat_mix`

## Expected Semantics by Case

- `s2i`: parses as integer and equals `123`.
- `s2f`: parses as finite float and equals numeric `3.5` (format may vary).
- `s2b_empty`: equals `false`.
- `s2b_text`: equals `true`.
- `f2i`: equals truncated integer from `12.75` (`12` expected).
- `i2f`: parses as finite float and equals numeric `42`.
- `i2s`: equals exact string `42`.
- `f2s`: parses as finite float and equals numeric `6.25` (format may vary).
- `b2s_true`: equals exact string `true`.
- `b2s_false`: equals exact string `false`.
- `concat_mix`: includes `A=7`, includes numeric `B=2.5` semantically, includes `C=true`, and contains no placeholder degradation.

## Result Summary

- AVR overall: PASS / FAIL
- ARM overall: PASS / FAIL
- Notes:
