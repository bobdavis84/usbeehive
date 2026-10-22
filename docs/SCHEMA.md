# Structured-output and D-Bus contracts

This document specifies the implemented serializers, not the enhancement brief.
The companion [type inventory](schema-types.json) contains the serialized core
USB, BOS, PD, Type-C, cable, diagnostic, snapshot, comparison, and history
struct/enum declarations. The M3 alternate-mode report and Thunderbolt-domain
additions are described in §5. Source links in the inventory are relative to this
directory. It is a machine-readable **type inventory**, not a JSON Schema
validator.

## 1. Formats are distinct

| Producer | JSON root | Naming / identity |
|---|---|---|
| CLI `--json` | array of legacy CLI device objects (§3) | camelCase; `category` is `usb`, `hub`, or `typec`; no top-level `id` |
| D-Bus `SnapshotJson` | array of serde `DeviceSummary` (§4) | snake_case; `category` is `UsbDevice`, `Hub`, or `TypeCPort`; `id()` is a method, not a serialized field |
| `Snapshot::to_json[_pretty]` | `Snapshot` envelope (§7) | snake_case; contains raw inventories and `summaries` |
| `compare::SnapshotDiff` serialization | comparison object (§8) | snake_case; `added`, `removed`, `modified` |
| History serialization | history / entry / timeline objects (§9) | snake_case; tagged transitions |
| CLI `--altmodes` | `AltModeReport` object (§5) | snake_case; system-wide USB4 signals are not assigned to a port |
| CLI `--watch --history --json` | one JSON object per capture (§9) | NDJSON; `timestampMs`, baseline flag, transition array |
| D-Bus `ListDevices` | typed tuple array (§10), not JSON | positional `DeviceEntry` |

In particular, `SnapshotJson` does **not** mirror CLI `--json`, despite the older
method comment in `src/dbus.rs`. A CLI JSON array is not a saved Snapshot envelope
and must not be passed to `Snapshot::from_json` as though it were one. Converting
an array to `{ "summaries": array }` is valid only for serde summaries, **not** for
legacy CLI objects.

### Stability and consumer rules

* Existing legacy CLI field names/types and Devices5 tuple positions are stable.
  Additional optional JSON members, property keys, and D-Bus signals are additive.
  Changing a tuple layout requires a new interface name (as with Devices4 → 5).
* Consumers must ignore unknown object members, property keys, and signals. String
  enums may grow: consumers should preserve unknown spellings or fall back to
  `Unknown`/neutral UI behavior. Removing or renaming existing values is breaking.
  Rust serde-derived enum deserialization itself is strict about unknown variants;
  tolerant external consumers must implement their own fallback.
* Human-readable `headline`, `subtitle`, summaries, details, labels, actions, notes,
  and property values containing formatted prose are not parsing interfaces.
* Raw sysfs attributes and new topology, comparison, history, structured-diagnostic,
  and trust inventories are additive/experimental contracts for this release.
  No schema-version member or `--experimental` switch is currently serialized.
  Save the producing package version alongside experimental data if persistence
  across releases is required. Do not invent a version member in Devices5 tuples.
* Ordering of devices and properties is discovery order, not a sorting guarantee.
  `usb:<bus_port>` and `typec:<port_name>` are topology identifiers, not globally
  unique hardware identities; moving ports can change identity.

## 2. Type mapping, units, and absence

For the inventory's Rust declarations: `String`, `&str`, and `PathBuf` serialize as
JSON strings; `bool` as boolean; unsigned/signed integer types as JSON integers
within the Rust type's range; `Vec<T>`, `VecDeque<T>`, and `[T; N]` as arrays;
`BTreeMap<String, String>` as an object of string values; `(String, String)` as a
two-element string array. `usize` is a nonnegative platform-sized integer. No
floating-point measurement is implied by an integer power or speed field.
Struct field names are literal snake_case except Rust `r#type` becomes `type`.

`Option<T>` serializes to `T` or JSON `null` unless its declaration explicitly has
`skip_serializing_if = "Option::is_none"`, in which case the absent key is omitted.
`#[serde(default)]` controls **reading** missing keys; deriving Rust `Default`
alone does not make every JSON member optional. Empty arrays/objects are emitted
unless a skip attribute says otherwise. Unit enum variants serialize using their
exact Rust spelling. Ordinary payload enums use serde's externally tagged form
unless overridden: `BosCapability` uses an internal `kind` tag; `StateTransition`
uses `transition_type` and `details`.

Unit suffixes are normative: `_mw` / `MW` = milliwatts, `_mv` / `MV` = millivolts,
`_ma` / `MA` = milliamps, `_uv` / `UV` = microvolts, `_ua` / `UA` = microamps,
`_mbps` / `Mbps` = megabits per second, `_ms` = milliseconds, `max_watts` /
`maxWatts` = watts. IDs, revisions, VDO bit fields, descriptor byte arrays, indices,
class codes, and lane counts are dimensionless. Other exceptional units are
stated in the inventory field documentation. Raw attributes retain kernel text
and kernel units; they are not normalized.

Zero is an existing sentinel for unknown speed, absent PD power/contract, missing
IDs, and some unreadable legacy raw attributes; it is not proof of a measured
zero. `-1` is the legacy unknown port/index sentinel where explicitly specified.
New `Option` telemetry distinguishes unknown from known false/zero. Missing
controller telemetry does not establish that a feature or cable e-marker is
physically absent. Power is advertised, requested, or inferred negotiated
capacity, **not** wattmeter telemetry. In particular, UCSI `current_now` may be
an RDO operating ceiling rather than a measurement of instantaneous current.

## 3. Legacy CLI `--json` object

Every array element has these required members:

| Members | JSON type | Meaning |
|---|---|---|
| `category` | string | `usb`, `hub`, `typec` |
| `deviceClass`, `deviceSubclass`, `status` | string | enums/hint below; subclass may be empty |
| `headline`, `subtitle`, `icon` | string | display text / freedesktop icon |
| `vendor`, `product`, `primaryDriver` | string | manufacturer/fallback, product, first bound driver; empty if unavailable |
| `vendorId`, `productId` | string | lowercase `0x` + four hexadecimal digits; `0x0000` for non-USB |
| `linkSpeedMbps` | integer u32 | negotiated speed, 0 unknown |
| `usbVersion` | string | canonical version, e.g. `2.0`, `3.2`; empty unknown |
| `properties` | array of `[string, string]` | extensible machine-key/value pairs; not a JSON object |
| `power` | object | `powerInMW`, `powerOutMW`, `contractMW`: u32 mW; `powerRole`: string enum |

`deviceClass`: `Keyboard`, `Mouse`, `InputTablet`, `Gamepad`, `Storage`, `Display`,
`Audio`, `Camera`, `VideoCapture`, `Printer`, `Phone`, `Hub`, `NetworkWired`,
`NetworkWireless`, `SecurityKey`, `SmartcardReader`, `Bluetooth`, `Serial`, `Unknown`.
`status`: `Empty`, `Connected`, `Charging`, `Sourcing`.
`powerRole`: `Source`, `Sink`, `DualRole`, `Unknown`.

The following objects are **omitted**, not null, when not applicable:

| Object | Members and types |
|---|---|
| `usb` | `vendorId`, `productId` hex strings; `manufacturer`, `product`, `speedLabel`, `version`, `serial`, `removable` strings; `speed` u32 Mbps; `maxPowerMA` u32 mA; `bus`, `device` integers; `isHub` boolean; `interfaces` array of `{class: string, driver: string}` |
| `usb.quirks` | numeric bitmask, only when nonzero; then `usb.bosSuppressedByQuirk` boolean is also present |
| `usb.bos` | serde `BosDescriptors` object (see inventory); omitted without BOS |
| `usb.raw`, `typec.raw` | string-to-string map, only with `--raw` |
| `dataRate` | serde `DataRateAssessment`: `verdict`, `negotiated_mbps`, `capable_mbps`, `functional_floor_mbps`, `capable_gen`, `summary`, `detail`, `is_warning` |
| `typec` | `port` i32; `dataRole`, `powerRole` selected-role strings (not raw bracket lists); `portType`, `powerOpMode` strings; `connected` boolean |
| `typec.powerSupply` | `name`, `chargeType`, `usbType` strings; `online` boolean; optional signed i64 `voltageNowUV`, `currentNowUA`, `currentMaxUA`, `voltageMinUV`, `voltageMaxUV`; optional `negotiatedPowerMW` integer. Unknown numeric members omitted individually |
| `cable` | `type`, `vendorName` strings; `vendorId` hex string; `maxWatts` u32; optional human-readable `speed`, `current` strings |
| `powerDelivery` | `maxPowerMW` u32 mW; `sourceCapabilities` array of `{type: string, voltageMV: u32, currentMA: u32, powerMW: u32, active: boolean}` |
| `charging` | `summary`, `detail` strings; `isWarning` boolean |

The legacy PDO `type` label is `Fixed`, `Battery`, `Variable`, `PPS`, or `Unknown`;
it is **not** serde `PdoType`'s `FixedSupply` / `VariableSupply` / `Pps` spelling.
Legacy CLI PDOs do not expose maximum voltage or a PDO index. `charging` is a
legacy projection, not the entire `ChargingDiagnostic` model. `dataRate` and
`usb.bos` retain snake_case nested serde fields even inside camelCase CLI output.
Cable speed/current strings are display labels, not serialized enum names.

## 4. Serde `DeviceSummary` (D-Bus `SnapshotJson`)

Required fields: `category`, `device_class`, `device_subclass`, `status`,
`headline`, `subtitle`, `icon`, `vendor`, `product`, `vendor_id` (u16 integer),
`product_id` (u16 integer), `primary_driver`, `properties`, `link_speed_mbps`,
`usb_version`, `power`, `charging_diag`, `data_rate`, `usb_device`, `typec_port`,
`power_delivery`, `cable`. Types/meanings match their inventory declarations.
`charging_diag`, `data_rate`, `usb_device`, `typec_port`, `power_delivery`, and
`cable` are nullable `Option` models.
No `id`, `port_number`, `pdo_list`, or `active_pdo_index` is injected at this level.

`power` is `{power_in_mw, power_out_mw, contract_mw, power_role}`. `charging_diag`
contains `bottleneck`, `summary`, `detail`, `is_warning`, `status`, `root_cause`,
`confidence`, `recommended_action`, and `rule_id`. The action is a **string**,
not an optional string, in the current implementation. Exact enum spellings:

* diagnostic status: `Healthy`, `Warning`, `Critical` (not lowercase);
* confidence: `High`, `Medium`, `Low` (categorical; no numeric score);
* root cause: `Cable`, `Charger`, `Sink`, `Port`, `Firmware`, `None`;
* bottleneck: `NoCharger`, `ChargerLimit`, `CableLimit`, `CableNoEMarker`,
  `DeviceLimit`, `SinkLimit`, `Fine`, `ContractOutOfSpec`;
* trust: `KnownGood`, `LikelyConstrained`, `Inconclusive`, `Suspicious`.

For `CableLimit`, a reported sink operating request strictly above the cable's
declared rating is stronger evidence than a capability comparison alone. If
that request is unavailable, an active source PDO above the cable rating yields
`Medium` confidence; if only the charger's advertised maximum is available,
confidence is `Low`. PDO power is a ceiling, not measured consumption; this
diagnostic does not certify a physical cable fault.

The request/rating boundary is compared in milliwatts without whole-watt
truncation: a 60,000 mW request on a 60 W cable does not by itself trigger a
cable warning, while 60,001 mW exceeds that declared rating.

`cable.trust` is `CableTrust`: booleans `zero_vid`, `vid_unknown`,
`reserved_bits_set`, `emarker_contract_mismatch`, `unsupported_combination`;
`trust_level` enum; `notes` string array. The separate `TrustAssessment` API model
contains `level`, `confidence`, `flags` (array of `{code, description, is_warning}`),
`summary`, `explanation`, `limitations_note`. Do not confuse the two types.

The summary's extensible `properties` list also carries `diagnostic.status`,
`diagnostic.root_cause`, `diagnostic.confidence`,
`diagnostic.recommended_action`, and `diagnostic.rule_id` whenever a charging
diagnostic exists. If cable telemetry exists, `cable.trust.level` carries the
`CableTrustLevel` spelling. These are string values, including empty action text
where the model supplies it; they are additive to the fixed Devices5 tuple and
the full serde `charging_diag` / `cable` objects.

Nested raw-model inventory entry points:

| Field | Type and recursive entry points in `schema-types.json` |
|---|---|
| `usb_device` | `usb::UsbDevice` → `UsbInterface`, `bos::BosDescriptors` → `BosCapability` and each capability payload |
| `typec_port` | `typec::TypeCPort` → `TypeCPowerSupply`, `TypeCPartner`, `TypeCCable` → `TypeCIdentity`, `TypeCAltMode`; alternate-mode view → `altmode::PortAltModes`, `DisplayPortInfo`, `ThunderboltAltModeInfo`, `OtherAltMode` |
| `power_delivery` | `power::PowerDeliveryPort` → `PowerDataObject`, `PdoType` |
| `cable` | `cable::CableInfo` → `CableTrust`, `pd::CableSpeed`, `CableCurrent` |
| `data_rate` | `bos::DataRateAssessment` → `DataRateVerdict` |
| snapshot routers/domains | `thunderbolt::ThunderboltRouter`, `ThunderboltDomain`; `snapshot::SourceAvailability` |

## 5. Alternate-mode / USB4 report and expanded snapshot

`--altmodes` emits the serialized `AltModeReport` object, or its dedicated
human-readable rendering. It has `ports: PortAltModes[]`, `billboards:
BillboardReport[]`, `usb4: Usb4Overview`, and `caveats: string[]`. `Reported<T>`
is tagged as `{"state":"not_applicable"}`, `{"state":"not_reported"}`, or
`{"state":"reported","value":...}`. This distinguishes a disconnected port
from a connected partner whose modes the kernel did not expose.

`PortAltModes` includes the Type-C name/number, `partner_connected`, DisplayPort
and Thunderbolt-3 reports, and other partner modes. `DisplayPortInfo` reports
whether a mode is active, its mode number, optional raw VDO, and optional decoded
capabilities. The DP decoder exposes roles, signaling bits, receptacle and USB2
flags, and advertised DFP_D/UFP_D pins. Those pins are **capabilities**, not the
active configuration; the latter is separately optional only if a kernel driver
publishes `displayport/pin_assignment`. `BillboardReport` gives the USB device's
bus-port, SVIDs, optional overall state and failure-reason strings.

`Usb4Overview` contains `thunderbolt_bus` (`present`, `absent`, `unknown`), domain
reports, a router topology forest, optional system-wide `usb4_link_active`, and an
explicit tunnel-visibility note. `DomainReport` describes the domain's security
level and its PCIe tunneling interpretation, IOMMU DMA protection, and
deauthorization support. `TopologyNode` includes domain/route/depth, protocol,
optional link authorization/NVM/speed/lane data, orphan status and children.
All absent readings remain `null` in this standalone report; empty optional
strings are not substituted for absent link attributes. A system-wide USB4 link
is never inferred to belong to a specific Type-C port because sysfs does not
provide that mapping. The kernel currently does not expose live per-tunnel state
or active DP lane configuration through the interfaces used here.

| Type | Serialized fields (all literal snake_case) |
|---|---|
| `AltModeReport` | `ports`, `billboards`, `usb4`, `caveats` |
| `PortAltModes` | `port_name: string`, `port_number: i32`, `partner_connected: bool`, `displayport: Reported<DisplayPortInfo>`, `thunderbolt: Reported<ThunderboltAltModeInfo>`, `other: OtherAltMode[]` |
| `Reported<T>` | externally tagged by `state`, payload key `value`; states `not_applicable`, `not_reported`, `reported` |
| `DisplayPortInfo` | `active: bool`, `mode: u32`, `vdo: u32|null`, `capabilities: DpCapabilities|null` |
| `DpCapabilities` | `raw: u32`, `port_capability: DpPortCapability`, `signaling_bits: u8`, `receptacle: bool`, `usb2_signaling_unused: bool`, advertised DFP_D/UFP_D pin-letter arrays |
| `ThunderboltAltModeInfo` | `active: bool`, `mode: u32`, `vdo: u32|null` |
| `OtherAltMode` | `svid: u16`, `mode: u32`, `active: bool`, `description: string|null` |
| `BillboardReport` | `bus_port: string`, `svids: u16[]`, `overall_state: string|null`, `failure_reasons: string[]` |
| `Usb4Overview` | `thunderbolt_bus: BusState`, `domains: DomainReport[]`, `topology: TopologyNode[]`, `usb4_link_active: bool|null`, `tunnels: TunnelVisibility` |
| `DomainReport` | `name: string`, `domain: u32`, `tunnel_policy: TunnelPolicy`, `iommu_dma_protection: bool|null`, `deauthorization: bool|null` |
| `TunnelPolicy` | `security: string|null`, `pcie_tunneling: bool|null`, `explanation: string` |
| `TopologyNode` | `route: string`, `domain/depth: u32`, `is_host: bool`, `generation: u8`, `protocol: string`, `vendor_name/device_name/unique_id: string|null`, `authorized: bool|null`, `nvm_version: string|null`, `rx_speed_gbps/tx_speed_gbps: u32|null`, `rx_lanes/tx_lanes: u8|null`, `orphan: bool`, `children: TopologyNode[]` |
| `TunnelVisibility` | `available: bool`, `note: string` |

`BusState` is the exact enum string `Present`, `Absent`, or `Unknown`.
`DpPortCapability` is `Reserved`, `UfpD`, `DfpD`, or `DfpDAndUfpD`; its separate
`label()` helper yields `reserved`, `UFP_D`, `DFP_D`, or `DFP_D+UFP_D` in
property/text presentation. DP pin assignments are advertised letters `A`–`F`,
not a lane-count measurement. Link speed is whole **Gb/s per lane**, lane fields
are lane counts. The raw JSON model and flattened `altmode.*` summary properties
are different projections; consumers should not synthesize the absent optional
values from the summary strings.

Saved `Snapshot` envelopes additionally include optional `availability`
(`usb_bus`, `typec_class`, `pd_class`, `thunderbolt_bus`) and
`thunderbolt_domains[]`. Missing `availability` in a legacy snapshot means
"not recorded," not absent. `TypeCAltMode` now has optional `reported_active`,
`pin_assignment`, `vdo`, and `description`; old `active: bool` remains. The
`ThunderboltRouter` legacy `generation: u8` still uses `0` when unknown, while
additive `reported_generation: Option<u8>` distinguishes unreadable generation;
optional authorization, NVM version and receive/transmit link fields preserve
the kernel's absence semantics. These models are best-effort and depend on
kernel, controller and driver support.

The inventory covers core declarations separately from the standalone M3 report
types above; public additions are source-linked in `src/altmode.rs`,
`src/snapshot.rs`, `src/thunderbolt.rs`, and `src/typec.rs`.

## 6. CLI filtering and reports

`--class`, `--vendor`, `--product`, `--id`, `--min-speed`, `--max-speed`,
`--charging`, `--port`, and `--severity` filter summaries. Distinct filters are
ANDed; repeated values of a selector are ORed. Unknown data does not match a
filter requiring that field. `--summary [--json]` returns counts, severity and
bottleneck rollups, cable issue count, up to ten highest-severity problems, and a
per-port overview. `--format csv|json|report` exports the matching inventory;
CSV quotes all fields. Filtering/reporting is binary-local and does not add
library filter APIs.

`--save-snapshot PATH` stores an unfiltered `Snapshot` envelope, separately from
rendering. `diff A [B|live] [--json]` consumes these envelopes (or compatible
serde snapshots with canonical summaries); unsupported legacy CLI JSON and
raw-only snapshots are rejected. `--watch --history --json` emits newline-
delimited objects with `sequence`, `timestampMs`, `baseline`, and `transitions`.
`--samples N` bounds capture; `--interval-ms` controls polling; the in-memory
buffer is bounded by `--history-capacity` and is not persisted automatically.

Unlike legacy `--raw` control, serde raw models include their `raw_attributes`
maps and `sysfs_path` strings as declared. They can reveal serial numbers and
local paths; redact before publishing diagnostic captures. BOS capabilities
are tagged with `kind`, e.g. `{"kind":"ContainerId","uuid":[16 byte integers]}`
(the bracket description is illustrative, not literal JSON). All detailed
capability fields, enum payloads, bits, and units are preserved in the inventory.

Trust is a heuristic, not certification. Missing e-marker telemetry, including
a passive cable whose identity was not published, is inconclusive: it does not
establish that the physical cable is unmarked or unsafe. An active PDO's maximum
current is a capability ceiling, not evidence that the cable is operating at
that current. Missing sysfs data, a stale vendor
registry, kernel omissions, and valid vendor behavior can produce false
positives. Cloned e-markers and physical wire defects can evade these checks.
Never interpret `KnownGood` as proof of electrical safety.

## 7. Saved `Snapshot` envelope

`usb_devices: UsbDevice[]`, `typec_ports: TypeCPort[]`,
`pd_ports: PowerDeliveryPort[]`, `thunderbolt_routers: ThunderboltRouter[]`,
`summaries: DeviceSummary[]` are arrays; missing arrays default to empty on input.
`metadata` is omitted when absent (also accepts null), otherwise:
`{captured_at_ms: u64, sequence: u64, source: string}`. Timestamp is UNIX epoch
milliseconds; sequence is capture order, source is caller-supplied. Metadata
members default on input. No timestamp is synthesized by serde. `{}` is accepted
as an empty snapshot, but `[]` is not. Raw inventories and summaries may be
independently populated; serialization does not validate or regenerate them.

## 8. Comparison JSON

Comparison is explicitly **summary-driven**. Raw inventories are not independently
diffed: a raw-only change that is not reflected in summaries produces no summary
transition. `compare(before, after)` yields `{added: DeviceSummary[], removed:
DeviceSummary[], modified: DeviceDifference[]}`. This differs from the older
sysfs manager's notification-oriented `SnapshotDiff` (ID lists).

`DeviceDifference` has required `id`, `headline`, `subtitle`; optional changed
members `status`, `power`, `pd_contract`, `link_speed_mbps`, `diagnostic`, `cable`,
`data_rate`; optional nonempty `property_changes` array. Absence means no recorded
change, not a null measurement. Every `ValueChange<T>` is `{from: T, to: T}`;
when T is optional either endpoint can be null. Nested difference fields are:

* `PowerDifference`: `power_in_mw`, `power_out_mw`, `contract_mw`, `power_role`,
  `usb_max_power_ma`.
* `PdContractDifference`: `contract_string`, `active_pdo_index`, `voltage_mv`,
  `current_ma`, `max_power_mw`. Active PDO here is a **zero-based list index**.
* `DiagnosticDifference`: `status`, `bottleneck`, `is_warning`, `root_cause`,
  `summary`, `recommended_action` (optional-string change endpoints).
* `CableDifference`: `trust_level`, `is_suspicious`, `max_watts`, `current_rating`,
  `speed`, plus nonempty `notes_added`, `notes_removed` string arrays.
* `DataRateDifference`: `verdict`, `is_warning`.
* `PropertyDifference`: `key: string`, `change: ValueChange<Option<String>>`.

All change members above are omitted if unchanged. Exact generic endpoint types
and enums are in the inventory; units follow §2. Adding/removing an entire
summary uses `added`/`removed`, not a modification with all-null endpoints.

## 9. History and transitions

`SnapshotHistory` serializes `{capacity: usize, total_pushed: u64,
entries: HistoryEntry[]}`; capacity is retention, not a claim that every older
snapshot remains. `HistoryEntry` contains `sequence: u64`, `timestamp_ms: u64`,
optional omitted `timestamp_str: string`, `snapshot: Snapshot`,
`transitions: StateTransition[]`. `TimelineEntry` has the same sequence/time
members and one `transition`, without an embedded snapshot. Times are caller
supplied milliseconds, typically UNIX epoch; sequence is monotonically assigned
capture order. This library serialization does not itself imply a CLI history
command or an on-disk history file format.

A transition is `{"transition_type":"PowerChanged","details":{...}}`.
Exact tags and payload types (all fields documented in the inventory) are:
`DeviceConnected` / `DeviceConnectedEvent`, `DeviceDisconnected` /
`DeviceDisconnectedEvent`, `LinkSpeedChanged` / `LinkSpeedChangedEvent`,
`PowerChanged` / `PowerChangedEvent`, `ContractRenegotiated` /
`ContractRenegotiatedEvent`, `BottleneckChanged` / `BottleneckChangedEvent`,
`CableWarningChanged` / `CableWarningChangedEvent`, `PortRoleChanged` /
`PortRoleChangedEvent`, `DataRateChanged` / `DataRateChangedEvent`.
Old/new fields are separate payload members, not `ValueChange` objects. Optional
payload fields serialize as null unless their inventory declaration says to omit.

## 10. D-Bus `org.usbeehive.Devices5`

Session-bus service `org.usbeehive.Devices`, path `/org/usbeehive/Devices`.
D-Bus codes: `s` string, `q` u16, `u` u32, `i` i32, `b` boolean, `a` array,
parentheses struct. No D-Bus field here is nullable.

| Wire type | Exact zvariant signature |
|---|---|
| `PowerEntry` | `(uuus)` |
| `DiagnosticEntry` | `(bsssb)` |
| `PdoEntry` | `(usuuuub)` |
| `DeviceEntry` | `(ssssssssssqqsa(ss)ius(uuus)(bsssb)a(usuuuub)i)` |
| `ListDevices` result | `a(ssssssssssqqsa(ss)ius(uuus)(bsssb)a(usuuuub)i)` |

`DeviceEntry` positional fields, in order:

| Position | Field | Signature / semantics |
|---|---|---|
| 1–10 | `id`, `category`, `device_class`, `device_subclass`, `status`, `headline`, `subtitle`, `icon`, `vendor`, `product` | ten strings; category uses Rust spellings |
| 11–12 | `vendor_id`, `product_id` | `q`, numeric IDs, zero for non-USB |
| 13 | `primary_driver` | `s`, empty if unbound |
| 14 | `properties` | `a(ss)`, extensible key/value bag |
| 15 | `port_number` | `i`, -1 without Type-C port |
| 16 | `link_speed_mbps` | `u`, 0 unknown |
| 17 | `usb_version` | `s`, empty unknown |
| 18 | `power` | `(uuus)`: `power_in_mw`, `power_out_mw`, `contract_mw`, `power_role` |
| 19 | `charging_diag` | `(bsssb)`: `present`, `bottleneck`, `summary`, `detail`, `is_warning` |
| 20 | `pdo_list` | `a(usuuuub)`: each `index`, `kind`, `voltage_mv`, `max_voltage_mv`, `current_ma`, `power_mw`, `is_active` |
| 21 | `active_pdo_index` | `i`, zero-based position in `pdo_list`, -1 without active PDO |

A PDO's own `index` is the kernel source-capabilities index (usually one-based),
not `active_pdo_index`. `kind` uses `Fixed`, `Battery`, `Variable`, `PPS`, `Unknown`.
`max_voltage_mv` is zero for fixed supplies. Diagnostic absence is represented by
`present=false`, empty strings, and `is_warning=false`, not a null struct.
Do not append structured diagnostics/trust fields to these tuples; use additive
property keys or the separately serialized models.

| Member | Input signature | Output signature |
|---|---|---|
| `ListDevices` | empty | DeviceEntry array above |
| `ListPorts` | empty | `ai` |
| `Diagnose` | `i` port number | `(bsssb)` |
| `SnapshotJson` | empty | `s` containing §4 array |
| `Refresh` | empty | `u` summary count |

Read-only properties: `Version: s` (package version), `DeviceCount: u`.
Signals: `DeviceAdded(ss)` (id, headline), `DeviceRemoved(s)` (id),
`DeviceChanged(s)` (id), `CapabilityDegraded(iss)` (port, summary, detail),
`CapabilityRestored(i)` (port), `DataRateDegraded(sss)` (id, summary, detail),
`DataRateRestored(s)` (id). Signal delivery is not persistent history: re-snapshot
on notification. `DeviceChanged` excludes raw power-magnitude jitter.
`tests/dbus_interface.rs` verifies zvariant signatures and generated introspection
without requiring a running session bus.

## 11. Consumer examples

```sh
# Legacy CLI JSON: IDs are hexadecimal strings; properties are pair arrays.
usbeehive --json | jq '.[] | {vendorId, productId, linkSpeedMbps, power}'
usbeehive --json | jq '.[] | .properties | map({key: .[0], value: .[1]}) | from_entries'
# A saved serde Snapshot is an object, not the legacy CLI array.
jq '.summaries[] | {category, vendor_id, power, charging_diag}' saved-snapshot.json
```

```python
# python-dbus: SnapshotJson returns JSON text, not a D-Bus array of dictionaries.
import json
import dbus
bus = dbus.SessionBus()
proxy = bus.get_object('org.usbeehive.Devices', '/org/usbeehive/Devices')
api = dbus.Interface(proxy, 'org.usbeehive.Devices5')
for summary in json.loads(str(api.SnapshotJson())):
    diagnostic = summary.get('charging_diag')
    if diagnostic and diagnostic.get('is_warning', False):
        print(summary['headline'], diagnostic.get('recommended_action', ''))
```

Consumers should branch on the **producer contract**, not heuristically rename
all camelCase keys to snake_case: both value types and selected nested content
differ between the legacy CLI and serde formats.
