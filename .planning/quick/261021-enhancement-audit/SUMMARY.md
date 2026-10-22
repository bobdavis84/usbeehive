# USBeehive enhancement completion summary

Initial implementation verification: 2026-10-21. Worktree branch:
`feature/enhancement-integration`. The user subsequently authorized committing
and delivering the integration to GitHub. No crates.io publication, tag, or
release version change is part of this delivery.

## Acceptance-criteria evidence matrix (28/28)

All 28 criteria below are implemented by integration commit
[`6eeb93a1038f4e90eaf341073fb37fb56dc1fd0d`](https://github.com/bobdavis84/usbeehive/commit/6eeb93a1038f4e90eaf341073fb37fb56dc1fd0d).
That commit also includes the sanitized mock exports, standalone DeviceCard,
expanded CI, and final regression fixes. M1/M2 references below identify the
inherited implementation history, not separate publication requirements.

| Req. | Acceptance criterion | Implementation/evidence | Status |
|---|---|---|---|
| R1 | Schema document exists | `docs/SCHEMA.md`, `docs/schema-types.json` | Met |
| R1 | README structured-output examples | README CLI/jq/consumer examples; schema §11 | Met |
| R1 | Breaking changes/migration notes | No wire tuple changes; Devices4→5 history retained; additive API compatibility caveats in schema and changelog | Met |
| R1 | D-Bus and JSON shapes documented consistently | Schema documents legacy CLI camelCase, serde `SnapshotJson`, saved snapshots, and Devices5 separately | Met |
| R2 | Clear diagnostic class and explanation | `ChargingDiagnostic` status/root cause/confidence/rule/action; CLI renderer and JSON | Met |
| R2 | Cable/charger/sink/port distinction | Structured root categories and classification tests | Met |
| R2 | CLI human-readable recommendation | list/tree/report renderers, CSV recommendation column, tests | Met |
| R2 | Diagnostic model documented | Schema §4; `diagnostic.rs` docs | Met |
| R3 | Sequential snapshots captured | `SnapshotHistory`; CLI `--watch --history`; `--samples` polling | Met |
| R3 | Recent transition summary/events | bounded history and per-capture transition events | Met |
| R3 | JSON and text transition output | NDJSON/text output; `tests/m4_cli_reports.rs` | Met |
| R3 | Past/current comparison | `compare` API and `diff A [B|live]`; summary-driven input limitation documented | Met |
| R4 | USB4/DP/TB alt modes identified | `altmode.rs` decoder/report and Type-C sysfs parsing | Met |
| R4 | Structured and text summaries | `--altmodes` JSON/text, flattened summary properties, Billboard report | Met |
| R4 | Missing data is explicit | `Reported<T>`, `Option` fields, source availability and caveats | Met |
| R4 | Hardware/kernel limits documented | README caveats and schema §5 | Met |
| R5 | Filters subsets of devices/ports | class, VID/PID, speed, charge state, port, severity filters | Met |
| R5 | Summary and export modes | `--summary`; CSV, JSON, report formats | Met |
| R5 | Output consistent/readable | default tree and existing JSON array retained; tests cover output contracts | Met |
| R5 | No downstream library filter API changes | filters/report are private CLI modules | Met |
| R6 | Trust states clear | trust level, notes, flags and structured serialization | Met |
| R6 | Uncertainty explained | missing identity is inconclusive; PDO ceiling not treated as operating current; regression tests | Met |
| R6 | README limitations and false positives/negatives | trust limitations, UCSI omissions, clone/physical limitations documented | Met |
| R6 | Structured trust in JSON/D-Bus | CLI `cableTrust`, serde CableTrust, `cable.trust.level` property | Met |
| R7 | Two snapshots diffable | public compare API and CLI saved/live comparisons | Met |
| R7 | Clear text and JSON differences | SnapshotDiff renderer and `--json`, including presence/removal cases | Met |
| R7 | Useful for troubleshooting/automation | semantic field-level diff and machine-readable output | Met |
| R7 | Examples/tests for common transitions | README library example; M2 and takeover regression suites | Met |

## Milestone and source map

- M1 diagnostics/trust baseline: existing commit `83cba01` (`feature/enhancement-diagnostics-1`); audited and amended in the current working tree for conservative missing-telemetry behavior and legacy serde compatibility.
- M2 snapshots/history/compare baseline: existing commit `7842dba` (`feature/enhancement-history-compare-2`); current working tree adds missing-contract, history-bound, and complete text-diff regressions/fixes.
- M3 USB4/Thunderbolt/alternate modes: consolidated integration changes; `src/altmode.rs`, `src/typec.rs`, `src/thunderbolt.rs`, `src/snapshot.rs`, `src/sysfs/{typec,thunderbolt,manager}.rs`, `src/summary.rs`; `tests/m3_usb4_altmodes.rs`.
- M4 filters/reports/snapshot save+diff/history CLI: consolidated integration changes; binary-local `src/{filter,severity,report}.rs`, `src/{main,output}.rs`; `tests/m4_cli_reports.rs`.
- M5 schema/D-Bus/README/changelog: `docs/SCHEMA.md`, `docs/schema-types.json`, `tests/dbus_interface.rs`, README, CHANGELOG.
- R8 audit: this directory's `PLAN.md`, this `SUMMARY.md`, and `verify_gates.py`.

## Compatibility notes

`org.usbeehive.Devices5` retains the pinned `DeviceEntry`, nested power/diagnostic/PDO tuple signatures and method/signal introspection. New structured diagnostic and trust values use additive `properties: a(ss)` keys. CLI legacy JSON remains a camelCase array; D-Bus `SnapshotJson` is a serde snake_case array; `--save-snapshot` writes a separate `Snapshot` envelope. Comparison/history operate on canonical summaries and do not independently diff raw top-level telemetry; the CLI rejects raw-only comparison input. USB4 is system-wide because the kernel sysfs tree does not map a router to a particular Type-C port.

## Verification

The offline feature matrix completed with exit code 0 for all 11 gates. `cargo fmt --all --check`, both default/all-feature clippy gates, both rustdoc gates, and the release D-Bus build passed. Tests passed with zero failures: default 550, core-only 323, CLI-only 545, D-Bus 440, and all-features 579. The reproducible runner and full logs/results are in `/home/bobdav/.hermes/cache/scratch/usbeehive-takeover/final-gates-reviewed-fixed/` (`results.json`). No-default and D-Bus test builds emitted `missing_docs` warnings for integration-test crate roots; clippy and rustdoc gates were clean.

Real CLI smoke checks covered live-read-only `--json`, `--altmodes`, `--summary`, CSV/report export, snapshot save, snapshot self-diff, a modified-snapshot diff, and bounded `--history --samples` without `--watch`. The live JSON parsed as an array (10 entries); snapshot self-diff found no changes and the altered snapshot found one changed Type-C port. A D-Bus daemon launched only inside `dbus-run-session`; `gdbus introspect` verified `org.usbeehive.Devices5` and `ListDevices` returned entries. No system-wide D-Bus daemon or device state was modified.

Independent review follow-up: the UCSI warning was not a measured-current inference—the parser specifically reads `ucsi-source-psy-*`, and Linux `drivers/usb/typec/ucsi/psy.c` maps PD `current_now` to the RDO operating current; this is now explicit in `src/sysfs/typec.rs`. The route-identity finding was fixed: route-shaped Thunderbolt entries without `generation` are retained only when both router `vendor` and `device` ABI attributes are readable; service-shaped entries without those markers are ignored. Bounded `--history --samples` no longer requires `--watch`, and the README example reflects this.

## GitHub delivery preparation

- The complete 11-gate offline matrix passed again before publication; see
  `/home/bobdav/.hermes/cache/scratch/usbeehive-takeover/publication-final-gates/results.json`.
  Final post-review counts: default 553, core-only 324, CLI-only 548, D-Bus 441,
  all features 582; zero failures in every configuration.
- `ui/` contains the standalone DeviceCard requested after the enhancement
  work. `npm ci`, `npm test` (4 tests), and `npm run typecheck` passed; the
  TypeScript check includes the tests, not just production component files.
- Root mock JSON files retain real capture structure with serials, UUIDs,
  container IDs and absolute sysfs paths privacy-sanitized. Raw originals remain
  outside the repository. See `docs/MOCK_DATA.md` for provenance and limitations.
- GitHub CI now runs the full Rust feature matrix, all-feature clippy/rustdoc,
  and DeviceCard tests/type checking. Remote CI and the eventual merge commit
  must be verified against their exact SHA before claiming GitHub delivery.
- Actual minimum-toolchain checks passed with the locked dependencies: Rust
  1.85.0 for the default CLI, and Rust 1.87.0 for the optional D-Bus build.
  The zbus stack requires 1.87; the README and CI now distinguish these minima.
- The final independent review identified and regression-tested two corrections:
  cable request/rating comparison now uses strict, overflow-safe milliwatts
  (59,999/60,000 mW are not cable overload; 60,001 mW is), and incompatible
  history/save and diff/history/altmode requests fail without emitting output.
- The existing M1/M2 commits are historical source references; M1–M5 and the
  post-review fixes are consolidated in the integration commit, not published
  as speculative per-milestone commit IDs.
- Original worktrees/backups are preserved. Their cleanup is not needed for
  publication and must not discard inherited work.
