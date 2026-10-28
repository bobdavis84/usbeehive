# USBeehive desktop

Linux desktop UI for the existing USBeehive Rust library. A browser preview renders the repository's **sanitized captured** D-Bus fixture; the Tauri window reads the **local machine** through the Rust crate. The preview is labelled demo and does not pretend to be live.

## Run

Requirements: Node 22+, Rust toolchain, `libudev` development files, GTK 3 and WebKit2GTK 4.1 development files (plus `pkg-config`). On Arch/CachyOS these are typically `base-devel`, `webkit2gtk-4.1`, `gtk3`, `systemd-libs` and `pkgconf`.

```sh
cd desktop
npm ci
npm run dev           # browser preview with sanitized fixture
npm run tauri dev     # native window and live sysfs
npm run build         # frontend production build
npm test              # model tests
npm run tauri build -- --debug  # native binary at src-tauri/target/debug/usbeehive-desktop
```

`npm run tauri build` makes an optimized native binary. Installer bundling is disabled for now. The app does not require `usbeehived`; it uses the crate's sysfs and watcher implementation directly. No root privilege is expected for ordinary enumeration, but what Linux exposes varies by controller and driver. The IBM Plex Sans/Mono files in `public/fonts/` are bundled for offline use under their accompanying `OFL.txt` license.

The native app reads local USB/Type-C/PD state. The changes view is an in-memory session history, not a persisted recorder. Data-rate comparison and charging diagnoses are evidence-based; requested/contract wattages are **not** measured consumption. No unsupported physical port attribution is inferred. The summary export uses a user-chosen save location and deliberately excludes serial numbers, raw sysfs attributes and paths, though names may still identify equipment and should be reviewed before sharing.

The UI uses the D-Bus `SnapshotJson` serde-summary shape, not the legacy CLI `--json` shape. `ui/DeviceCard` remains a separate standalone component.
