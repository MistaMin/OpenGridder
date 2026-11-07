# Client contract continuation — 2026-11-07

The client now rejects a malformed console state without partly applying its earlier fields. Both AG1 tokens and JSON console fields are validated before any parameter changes. AG1 NaN and infinities are rejected instead of being accepted as ignored parameter writes.

This tree has uncommitted changes and no Git repository. The parent owns native formats and the new live transport. This task changed only `apply_console_json` and the AG1 branch of `load_state` in `crates/ag-plugin/src/lib.rs`, plus `crates/ag-plugin/tests/client_contract.rs` and this evidence folder.

## Reproduction and result

`cargo test -p ag-plugin --test client_contract --offline` before the fix: 3 passed, 1 failed. Loading `AG1 input=12 unknown=3` returned false but changed input gain from −7 dB to +12 dB. See `client-contract-before.log`.

The same command after the fix: 4 passed, 0 failed. See `client-contract-after.log`. No assertion was removed, ignored, or loosened.

`cargo test -p ag-plugin --lib --offline`: 51 passed, 0 failed. See `client-lib-after.log`. This includes the parent's in-flight SPSC changes present at test time; it is a working-tree test, not a frozen release binary.

## Evidence mapping

| Ledger rows | New evidence |
|---|---|
| C1, C3 | bypass 0.49 versus 0.5/1.0; +24 dB output trim affects active audio and is skipped by bypass |
| C7, C9 | default latency 256; impulse delays exactly 128/256/512/1024 for profiles 0/1/2/3; helper fallback outside the profile range |
| C17 | output gain +6 dB measured on the delayed impulse |
| C18 | after saturation, +6 dB output is the untrimmed saturated result times 10^(6/20), over every sample after alignment |
| C21 | 50% mix of 0.25 dry and 0.75 remote is 0.5, before +6 dB output |
| C67, C69 | all nine console fields round-trip in version-7 JSON and legacy AG1 |
| New rejection regression | unknown token, invalid number, NaN, infinity, and late malformed JSON field all reject with byte-identical saved state |

These are processor harness checks. They do not verify native GUI controls, DAW state reopening, physical listening, or network behavior. The primary ledger has not been edited by this task.

## Remaining observations

- `process_f64` calls `process_with_remote` directly, whereas `Plugin::process` first checks the offline wet mailbox and updates the underrun counter. Plain f64 processing therefore does not share those two behaviors. Parent is integrating a new tagged live path and should account for the distinction.
- JSON chain entries with malformed fields are currently defaulted/skipped by the existing importer. This task deliberately preserved that compatibility policy and only changed rejection paths which already return false.
