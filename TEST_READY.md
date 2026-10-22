# TEST_READY — Test Suite Verification & Coverage Matrix (Milestone M_E2E)

- **Status**: READY
- **Test Suite**: `tests/e2e_suite.rs` (with modules in `tests/e2e/`)
- **Architecture Spec**: `TEST_INFRA.md`
- **Execution Pass Rate**: 81/81 passed (100%)
- **Verification Date**: 2026-10-21

---

## 1. Executive Summary

Milestone **M_E2E** is complete. A comprehensive, requirement-driven, opaque-box integration and end-to-end test suite has been designed, authored, and verified for `usbeehive`.

The test suite exercises all seven core requirements (**R1** through **R7**) across four rigorous testing tiers:
- **Tier 1 (Feature Coverage / Happy Path)**: 35 tests (5 per requirement area).
- **Tier 2 (Boundary & Corner Cases)**: 35 tests (5 per requirement area).
- **Tier 3 (Cross-Feature Combinations)**: 6 tests covering pairwise subsystem interactions.
- **Tier 4 (Real-World Scenarios)**: 5 complex multi-device topologies.
- **Total Test Count**: **81 tests**, executed via `cargo test --test e2e_suite`.

All tests execute in complete isolation using synthetic sysfs file trees created by the test harness (`tests/e2e/harness.rs`), ensuring deterministic behavior with zero dependence on host hardware or kernel state.

---

## 2. Test Coverage Matrix

| Req | Description | Tier 1 (Feature) | Tier 2 (Boundary) | Tier 3 (Cross-Feature) | Tier 4 (Scenario) | Total | Status |
|---|---|:---:|:---:|:---:|:---:|:---:|:---:|
| **R1** | JSON Output Schema & Structure | 5 tests | 5 tests | 2 tests | 2 tests | **14** | PASS |
| **R2** | Diagnostic Reasoning & Actionability | 5 tests | 5 tests | 2 tests | 3 tests | **15** | PASS |
| **R3** | History & State Transitions | 5 tests | 5 tests | 2 tests | 1 test | **13** | PASS |
| **R4** | Snapshot Comparison Tooling | 5 tests | 5 tests | 2 tests | 1 test | **13** | PASS |
| **R5** | CLI Output Modes & Filtering | 5 tests | 5 tests | 1 test | 2 tests | **13** | PASS |
| **R6** | USB4 / Thunderbolt / Alt-Modes | 5 tests | 5 tests | 2 tests | 2 tests | **14** | PASS |
| **R7** | Cable Authenticity & Trust | 5 tests | 5 tests | 2 tests | 2 tests | **14** | PASS |
| **Total** | | **35** | **35** | **6** | **5** | **81** | **PASS (100%)** |

---

## 3. Detailed Test Catalog

### Tier 1: Feature Coverage (35 Tests)
- **R1 (JSON Schema)**:
  - `t1_r1_json_empty_tree_returns_empty_array`: CLI `--json` emits empty array `[]` on empty sysfs root.
  - `t1_r1_json_usb_device_fields`: Validates `category`, `headline`, `vendorId`, `productId`, `speedMbps`, `maxPowerMA`.
  - `t1_r1_json_typec_charging_output`: Validates Type-C status `"Charging"`, `charging.summary`, `charging.isWarning`.
  - `t1_r1_json_multi_device_array`: Multi-device topology serialized as proper top-level JSON array.
  - `t1_r1_json_raw_flag_includes_sysfs_attrs`: CLI `--raw` flag exposes raw sysfs string map.
- **R2 (Diagnostic Reasoning)**:
  - `t1_r2_cable_limit_bottleneck_warning`: 100W charger + 60W cable triggers `CableLimit` warning.
  - `t1_r2_fine_unconstrained_charging`: 100W charger + 250W cable resolves to `Bottleneck::Fine`.
  - `t1_r2_device_limit_policy`: Negotiated power <80% of charger max triggers `DeviceLimit`.
  - `t1_r2_sink_limit_policy`: Sink requested power <80% of active contract triggers `SinkLimit`.
  - `t1_r2_cable_no_emarker_pinned_3a`: Request pinned at 3A against >3A PDO without e-marker triggers `CableNoEMarker`.
- **R3 (History & Transitions)**:
  - `t1_r3_sequential_snapshots_capture`: Captures sequence of distinct snapshots across time.
  - `t1_r3_device_arrival_transition`: Detects transition from empty to device connected.
  - `t1_r3_device_removal_transition`: Detects transition from device connected to disconnected.
  - `t1_r3_cable_degradation_transition`: Detects cable swap causing power bottleneck (`newly_degraded`).
  - `t1_r3_cable_restoration_transition`: Detects cable upgrade resolving bottleneck (`resolved`).
- **R4 (Snapshot Compare)**:
  - `t1_r4_identical_snapshots_diff_empty`: Identical topologies yield empty added/removed/changed sets.
  - `t1_r4_added_device_detection`: Flags newly attached peripheral in `diff.added`.
  - `t1_r4_removed_device_detection`: Flags detached peripheral in `diff.removed`.
  - `t1_r4_property_change_detection`: Flags role changes and property transitions in `diff.changed`.
  - `t1_r4_bidirectional_diff_symmetry`: Inverting diff order swaps `added` and `removed`.
- **R5 (CLI Modes & Filtering)**:
  - `t1_r5_cli_default_tree_rendering`: Default tree mode renders indented hierarchy and hub connectors.
  - `t1_r5_cli_list_mode_rendering`: `--list` renders flat list with headline and subtitle.
  - `t1_r5_filter_by_vendor`: Filters devices matching hex vendor ID.
  - `t1_r5_filter_by_class`: Filters devices matching USB interface class (e.g. HID 0x03).
  - `t1_r5_filter_by_category`: Filters summaries by category (`typec`, `hub`, `peripheral`).
- **R6 (USB4 / Thunderbolt / Alt-Modes)**:
  - `t1_r6_usb4_host_router_detection`: Recognizes root host router with `generation == 4`.
  - `t1_r6_usb4_device_router_detection`: Recognizes device router with `generation == 4`.
  - `t1_r6_usb4_active_link`: Detects active link when both host and device routers are present.
  - `t1_r6_tbt3_router_distinction`: Distinguishes Thunderbolt 3 router (`generation == 3`) from USB4.
  - `t1_r6_daisy_chain_topology`: Supports multi-hop router hierarchy (`0-0` -> `0-1` -> `0-1-1`).
- **R7 (Cable Authenticity & Trust)**:
  - `t1_r7_clean_cable_trust`: Certified e-marked cable with known VID evaluates to `KnownGood`.
  - `t1_r7_zero_vid_counterfeit`: Cable advertising VID 0x0000 flags counterfeit trust warning.
  - `t1_r7_unknown_vid_fallback`: Cable with unlisted VID falls back gracefully with advisory.
  - `t1_r7_reserved_dirty_bits`: Cable VDO with reserved bits set flags invalid specification bits.
  - `t1_r7_contract_rating_mismatch`: 5A contract negotiated on a 3A cable flags safety mismatch.

### Tier 2: Boundary & Corner Cases (35 Tests)
- **R1 (Boundary JSON)**: Empty sysfs root, missing optional attributes, extreme link speeds (80 Gbps / 0 Mbps), non-ASCII/special character product names, missing USB/Type-C class directories.
- **R2 (Boundary Diagnostics)**: Zero-wattage charger PDO, 0mA request handled safely, orphan PD port without partner, no source capabilities returns None, exact 80% threshold boundary condition.
- **R3 (Boundary History)**: Empty to empty transitions, role inversion between snapshots, rapid multi-device churn, subsystem restart resilience, oscillation recovery.
- **R4 (Boundary Diff)**: Empty vs populated diff, populated vs empty diff, completely disjoint topologies, partner detached while port remains, unrelated property change produces zero chatter.
- **R5 (Boundary Filtering & Modes)**: Filter matching zero devices, filter matching all devices, CLI conflicting flags handling, deep hub chaining tree depth, tree legend on empty tree.
- **R6 (Boundary USB4)**: Missing thunderbolt subsystem directory, router without generation attribute, router with generation 0, single host router with no downstream link, single device router with no host.
- **R7 (Boundary Cable Trust)**: Cable node without identity VDOs, passive cable without e-marker, cable with dirty reserved bits only, unknown VID with clean VDO, zero VID with clean VDO.

### Tier 3: Cross-Feature Combinations (6 Tests)
- `t3_combo_history_state_transition_with_cable_swap`: Combines History (R3) + Diagnostics (R2) + Cable Trust (R7) across sequential snapshots.
- `t3_combo_usb4_topology_with_degraded_downstream_link`: Combines USB4 hierarchy (R6) + BOS data rate assessment + JSON output (R1).
- `t3_combo_power_contract_renegotiation_diff`: Combines PD charger renegotiation + Snapshot Diff (R4) + Diagnostics (R2).
- `t3_combo_snapshot_diff_targeted_by_class`: Combines Snapshot Diff (R4) + Class Filtering (R5).
- `t3_combo_billboard_altmode_with_power_diagnostic`: Combines Type-C Alt-Mode / Billboard + Power Delivery charging diagnostics (R2 + R6).
- `t3_combo_json_cable_trust_diagnostic_pipeline`: Full-pipeline test: JSON output (R1) + Cable Trust (R7) + Charging Bottleneck (R2).

### Tier 4: Real-World Scenarios (5 Tests)
- `t4_scenario_usb_c_multi_port_dock`: Laptop connected to a 100W PD dock with 4-port hub and 3 downstream peripherals (Keyboard, Mouse, SuperSpeed Storage).
- `t4_scenario_laptop_on_undersized_15w_charger`: Laptop requesting 45W connected to a 15W phone charger with a 60W cable (`Bottleneck::ChargerLimit`, `CHARGER_UNDERPOWERED`).
- `t4_scenario_usb4_daisy_chain`: Dual-hop USB4 / TB4 daisy chain (Intel Maple Ridge host -> CalDigit TS4 dock -> OWC Envoy Pro SSD).
- `t4_scenario_counterfeit_cable_bottleneck`: 100W charger paired with a counterfeit 60W cable advertising VID 0x0000 and reserved dirty bits.
- `t4_scenario_hotplug_life_cycle_recovery`: Complete 4-stage hotplug lifecycle (Initial state -> Device connected -> Cable degraded -> Clean disconnect).

---

## 4. Expected Output Derivation & Oracle Grounding

All test cases are derived strictly from authoritative domain specifications:
1. **USB-PD Specification (Rev 3.1)**:
   - Fixed supply PDO encoding (`voltage_mv`, `current_ma`, `power_mw`).
   - Cable VDO bit fields: Voltage bits (`0 -> 20V`, `3 -> 50V`), current bits (`1 -> 3A`, `2 -> 5A`), and reserved bits (`bit 3`, `bit 7`, `bit 8`).
   - E-Marker requirements: Power contracts exceeding 3A (60W) require a certified 5A e-marked cable.
2. **USB 3.2 & USB4 / Thunderbolt Specifications**:
   - Link speeds: 12 Mbps (Full Speed), 480 Mbps (High Speed), 5000 Mbps (SuperSpeed Gen1), 10000 Mbps (SuperSpeed+ Gen2), 20000/40000/80000 Mbps (USB4 Gen3/Gen4).
   - Router generation attribute: `generation >= 4` defines USB4 / Thunderbolt 4 routers; `generation == 3` denotes Thunderbolt 3.
3. **Linux Kernel Sysfs ABI (`Documentation/ABI/testing/sysfs-class-typec`)**:
   - Attribute file layouts and format strings (`voltage_now`, `current_now`, `data_role`, `power_role`, `power_operation_mode`).
4. **Project Requirements**:
   - Exact CLI JSON fields, exit codes, filtering flags, and diagnostic rules defined in `ORIGINAL_REQUEST.md` and `PROJECT.md`.

---

## 5. Execution Guide

### Run the Complete E2E Suite
```bash
cargo test --test e2e_suite
```

### Run by Tier
```bash
cargo test --test e2e_suite tier1_features
cargo test --test e2e_suite tier2_boundaries
cargo test --test e2e_suite tier3_combinations
cargo test --test e2e_suite tier4_scenarios
```

### Run by Requirement Area
```bash
cargo test --test e2e_suite r1_json
cargo test --test e2e_suite r2_diagnostic
cargo test --test e2e_suite r3_history
cargo test --test e2e_suite r4_compare
cargo test --test e2e_suite r5_filter
cargo test --test e2e_suite r6_usb4
cargo test --test e2e_suite r7_trust
```

### Run Full Project Validation
```bash
cargo fmt --all --check
cargo clippy --test e2e_suite --all-features -- -D warnings
cargo test
```

---

## 6. Verification Results

```
     Running tests/e2e_suite.rs (target/debug/deps/e2e_suite-85300d19479d647c)

running 81 tests
test e2e::tier1_features::t1_r1_json_empty_tree_returns_empty_array ... ok
test e2e::tier1_features::t1_r1_json_multi_device_array ... ok
test e2e::tier1_features::t1_r1_json_raw_flag_includes_sysfs_attrs ... ok
test e2e::tier1_features::t1_r1_json_typec_charging_output ... ok
test e2e::tier1_features::t1_r1_json_usb_device_fields ... ok
test e2e::tier1_features::t1_r2_cable_limit_bottleneck_warning ... ok
test e2e::tier1_features::t1_r2_cable_no_emarker_pinned_3a ... ok
test e2e::tier1_features::t1_r2_device_limit_policy ... ok
test e2e::tier1_features::t1_r2_fine_unconstrained_charging ... ok
test e2e::tier1_features::t1_r2_sink_limit_policy ... ok
test e2e::tier1_features::t1_r3_cable_degradation_transition ... ok
test e2e::tier1_features::t1_r3_cable_restoration_transition ... ok
test e2e::tier1_features::t1_r3_device_arrival_transition ... ok
test e2e::tier1_features::t1_r3_device_removal_transition ... ok
test e2e::tier1_features::t1_r3_sequential_snapshots_capture ... ok
test e2e::tier1_features::t1_r4_added_device_detection ... ok
test e2e::tier1_features::t1_r4_bidirectional_diff_symmetry ... ok
test e2e::tier1_features::t1_r4_identical_snapshots_diff_empty ... ok
test e2e::tier1_features::t1_r4_property_change_detection ... ok
test e2e::tier1_features::t1_r4_removed_device_detection ... ok
test e2e::tier1_features::t1_r5_cli_default_tree_rendering ... ok
test e2e::tier1_features::t1_r5_cli_list_mode_rendering ... ok
test e2e::tier1_features::t1_r5_filter_by_category ... ok
test e2e::tier1_features::t1_r5_filter_by_class ... ok
test e2e::tier1_features::t1_r5_filter_by_vendor ... ok
test e2e::tier1_features::t1_r6_daisy_chain_topology ... ok
test e2e::tier1_features::t1_r6_tbt3_router_distinction ... ok
test e2e::tier1_features::t1_r6_usb4_active_link ... ok
test e2e::tier1_features::t1_r6_usb4_device_router_detection ... ok
test e2e::tier1_features::t1_r6_usb4_host_router_detection ... ok
test e2e::tier1_features::t1_r7_clean_cable_trust ... ok
test e2e::tier1_features::t1_r7_contract_rating_mismatch ... ok
test e2e::tier1_features::t1_r7_reserved_dirty_bits ... ok
test e2e::tier1_features::t1_r7_unknown_vid_fallback ... ok
test e2e::tier1_features::t1_r7_zero_vid_counterfeit ... ok
test e2e::tier2_boundaries::t2_r1_boundary_device_missing_optional_attrs ... ok
test e2e::tier2_boundaries::t2_r1_boundary_empty_sysfs_root ... ok
test e2e::tier2_boundaries::t2_r1_boundary_extreme_speed_values ... ok
test e2e::tier2_boundaries::t2_r1_boundary_missing_usb_typec_dirs ... ok
test e2e::tier2_boundaries::t2_r1_boundary_special_characters_in_names ... ok
test e2e::tier2_boundaries::t2_r2_boundary_exact_80pct_threshold ... ok
test e2e::tier2_boundaries::t2_r2_boundary_no_source_capabilities_returns_none ... ok
test e2e::tier2_boundaries::t2_r2_boundary_orphan_pd_port_without_partner ... ok
test e2e::tier2_boundaries::t2_r2_boundary_zero_ma_request_handled_safely ... ok
test e2e::tier2_boundaries::t2_r2_boundary_zero_watt_charger_returns_none ... ok
test e2e::tier2_boundaries::t2_r3_boundary_empty_to_empty ... ok
test e2e::tier2_boundaries::t2_r3_boundary_oscillation_recovery ... ok
test e2e::tier2_boundaries::t2_r3_boundary_rapid_multi_device_change ... ok
test e2e::tier2_boundaries::t2_r3_boundary_role_inversion ... ok
test e2e::tier2_boundaries::t2_r3_boundary_subsystem_restarts ... ok
test e2e::tier2_boundaries::t2_r4_boundary_diff_completely_disjoint ... ok
test e2e::tier2_boundaries::t2_r4_boundary_diff_empty_vs_populated ... ok
test e2e::tier2_boundaries::t2_r4_boundary_diff_partner_detached ... ok
test e2e::tier2_boundaries::t2_r4_boundary_diff_populated_vs_empty ... ok
test e2e::tier2_boundaries::t2_r4_boundary_diff_unrelated_property_no_chatter ... ok
test e2e::tier2_boundaries::t2_r5_boundary_cli_conflicting_flags ... ok
test e2e::tier2_boundaries::t2_r5_boundary_deep_hub_chain_tree ... ok
test e2e::tier2_boundaries::t2_r5_boundary_filter_all_matches ... ok
test e2e::tier2_boundaries::t2_r5_boundary_filter_zero_matches ... ok
test e2e::tier2_boundaries::t2_r5_boundary_tree_legend_empty_tree ... ok
test e2e::tier2_boundaries::t2_r6_boundary_missing_thunderbolt_subsystem ... ok
test e2e::tier2_boundaries::t2_r6_boundary_router_generation_zero ... ok
test e2e::tier2_boundaries::t2_r6_boundary_router_without_generation ... ok
test e2e::tier2_boundaries::t2_r6_boundary_single_usb4_device_no_host ... ok
test e2e::tier2_boundaries::t2_r6_boundary_single_usb4_host_no_link ... ok
test e2e::tier2_boundaries::t2_r7_boundary_cable_no_vdos ... ok
test e2e::tier2_boundaries::t2_r7_boundary_dirty_bits_only ... ok
test e2e::tier2_boundaries::t2_r7_boundary_passive_non_emarked_cable ... ok
test e2e::tier2_boundaries::t2_r7_boundary_unknown_vid_with_clean_vdo ... ok
test e2e::tier2_boundaries::t2_r7_boundary_zero_vid_with_clean_vdo ... ok
test e2e::tier3_combinations::t3_combo_billboard_altmode_with_power_diagnostic ... ok
test e2e::tier3_combinations::t3_combo_history_state_transition_with_cable_swap ... ok
test e2e::tier3_combinations::t3_combo_json_cable_trust_diagnostic_pipeline ... ok
test e2e::tier3_combinations::t3_combo_power_contract_renegotiation_diff ... ok
test e2e::tier3_combinations::t3_combo_snapshot_diff_targeted_by_class ... ok
test e2e::tier3_combinations::t3_combo_usb4_topology_with_degraded_downstream_link ... ok
test e2e::tier4_scenarios::t4_scenario_counterfeit_cable_bottleneck ... ok
test e2e::tier4_scenarios::t4_scenario_hotplug_life_cycle_recovery ... ok
test e2e::tier4_scenarios::t4_scenario_laptop_on_undersized_15w_charger ... ok
test e2e::tier4_scenarios::t4_scenario_usb_c_multi_port_dock ... ok
test e2e::tier4_scenarios::t4_scenario_usb4_daisy_chain ... ok

test result: ok. 81 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
```

---

## 7. Discovered Defects & Escalations

1. **Clippy Violations in Peer Test File (`tests/m1_stress_suite.rs`)**:
   - `tests/m1_stress_suite.rs` contains 7 clippy violations (`unused-imports`, `bool-assert-comparison`, `len-zero`).
   - Per subagent boundary rules, this file is not modified by this test writer agent.
   - **Escalation**: The implementing agent for M1 / M1 test author should clean up these 7 warnings in `tests/m1_stress_suite.rs` so that `cargo clippy --all-targets -- -D warnings` passes crate-wide.
