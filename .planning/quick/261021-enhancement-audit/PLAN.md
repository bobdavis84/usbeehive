# USBeehive enhancement takeover and verification plan

## Sources of truth

- `/home/bobdav/Documents/usbeehive_enhancement_brief.md` (seven user-facing enhancement areas).
- `.agents/teamwork/ORIGINAL_REQUEST.md` (R1–R8 implementation/verification requirements).
- `AGENTS.md` (architecture, feature hierarchy, quality gates).
- `.agents/teamwork/PROJECT.md` and the three initial survey reports (historical design suggestions, not authoritative descriptions of current behavior).
- Handoff attachment `/home/bobdav/.hermes/attachments/usbeehive_handoff.md`.

## Verified takeover state

- Main checkout was `master` at `3036f90` with uncommitted M1/M2 files. Comparing every file tracked by M2 against the main checkout, including files untracked on master, found **no content differences**.
- M1 is committed as `83cba01`; M2 as `7842dba`.
- Original `/tmp/usb-m3` and `/tmp/usb-m4` contain partial M3/M4 work. Preserve these rather than reset, delete, or force-switch them.
- Tracked binary diffs and untracked-file archives were saved under `/home/bobdav/.hermes/cache/scratch/usbeehive-takeover/` before any changes.
- New isolated worktrees `.agents/worktrees/m3` (`feature/hermes-m3`) and `.agents/worktrees/m4` (`feature/hermes-m4`) start at M2 and contain copies of the corresponding inherited changes. The original worktrees remain untouched. A direct worktree move was not possible across filesystems.
- `cargo test --offline` passed on the inherited main checkout. This is a baseline only; final tests must run again after integration.
- The historical `gates.sh` uses pipelines without `pipefail`, and can hide cargo failures. Replace its verification role with a runner that preserves each command's exit status and complete logs.

## Parallel ownership and execution

1. **M3**: Complete and test pure alternate-mode/DP decoder/report, Thunderbolt domains and optional links, honest availability, sysfs wiring and fixture coverage. Verify actual Linux route encoding, not speculative survey route examples. Do not infer per-port USB4 from a system-wide signal.
2. **M4**: Complete binary-local filter/severity/report modules, additive CLI flags, summary/CSV/report, saved snapshots and file/live comparison, opt-in watch/history events, and expose inherited structured diagnosis/trust in the CLI renderer. Preserve legacy default tree and JSON array.
3. **M5 wire/schema**: Pin Devices5 nested tuple signatures and generated interface surface; document all output contracts and their differences. Additive properties must not alter the tuple.
4. **Independent inherited-core audit**: Review M1/M2 diagnostic, trust, comparison, and history behavior for concrete bugs or missing requirement coverage.
5. **Integration owner**: Integrate work without overwriting other changes; add CLI alternate-mode report wiring, documentation/consumer examples, audit and acceptance matrix, and regression fixes demonstrated by failing tests.
6. **Independent final review and M6**: Review integrated code, resolve actionable findings, then execute all gates and real read-only CLI/D-Bus smoke tests.

## Compatibility boundaries

- Keep pure core IO-free, serde-only; sysfs reads remain in sysfs readers; filter/report logic stays private to CLI.
- Preserve legacy camelCase CLI `--json` array and distinguish it from snake_case D-Bus `SnapshotJson` (the old README incorrectly calls them identical), saved snapshots, semantic diffs, and history events.
- Preserve `org.usbeehive.Devices5` tuple positions/signatures. Add fields through the extensible properties bag.
- Document public Rust struct additions honestly: serde defaults preserve input compatibility but do **not** preserve exhaustive downstream struct literal construction.
- Distinguish advertised capability, negotiated/requested ceilings, and measured consumption. Missing telemetry is not evidence of defective hardware or absent capability.
- No release bump, commit, push, tag, publishing, destructive cleanup, or modification of original worktrees without explicit user authorization.

## Final gates (actual exit codes required)

Run offline with full logs:

- `cargo fmt --all --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test`
- `cargo test --no-default-features`
- `cargo test --no-default-features --features cli` (CLI without libudev)
- `cargo test --no-default-features --features dbus`
- `cargo test --all-features`
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`
- `cargo build --release --no-default-features --features dbus`

Inspect CLI help; exercise filters/exports, snapshot capture, self-diff and changed fixtures, unknown/missing sysfs, malformed files, and structured diagnosis/trust. Exercise optional D-Bus introspection on an isolated session bus if available, with no system-wide daemon installation.

## Deliverables

- Completed enhancements in the primary checkout, with inherited work preserved.
- `docs/SCHEMA.md`, README usage/consumer examples and CHANGELOG migration notes.
- Tests covering all seven enhancement areas plus feature matrix and wire stability.
- `SUMMARY.md`, reproducible verification runner/log manifest, and acceptance-criterion evidence matrix. Commit references identify existing M1/M2 commits; new uncommitted work must be labeled as such rather than inventing commit IDs.
