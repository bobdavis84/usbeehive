# Public mock payloads

The project-root fixtures are derived from real USBeehive output, not simulated
API responses:

- `mock-data.json`: CLI `usbeehive --json` (legacy camelCase projection).
- `mock-data-dbus.json`: the JSON string returned by the
  `org.usbeehive.Devices5.SnapshotJson` method, decoded out of the D-Bus reply
  (snake_case serde summaries). The method was called on an isolated session bus.

Both are arrays with 10 captured entries. The original capture did not include
any active charging diagnostic, so every `charging_diag` in the D-Bus fixture is
`null`; the CLI omits its inapplicable `charging` member. The populated diagnostic
in `ui/DeviceCard.test.tsx` is explicitly synthetic test data, not a claimed live
capture.

Before publication, serial numbers were replaced with `REDACTED`, UUID/container
identifiers were zeroed, and `sysfs_path` fields were replaced with
`/sys/mock-device`. These retain the original JSON field names, types, and device
relationships; they are privacy-sanitized snapshots, not byte-for-byte raw dumps.
Device model names and hardware capabilities remain unchanged. Do not use the
redacted paths or UUIDs as live device identifiers.

`ui/DeviceCard.tsx` consumes the D-Bus fixture's `headline`, `subtitle`, and
`charging_diag` shape. It does not connect to D-Bus or invoke the Rust library.
See [SCHEMA.md](SCHEMA.md) for the complete output contracts.
