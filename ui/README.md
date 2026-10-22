# DeviceCard

A standalone React/TypeScript presentational component for the D-Bus
`SnapshotJson` summary shape. It consumes only the `headline`, `subtitle`, and
nullable `charging_diag` fields from a caller-provided object; it makes no D-Bus,
Rust-library, or network calls.

Import the component and stylesheet through `index.ts`:

```tsx
import { DeviceCard, type DeviceCardData } from "./ui/index";
import mockData from "./mock-data-dbus.json";

const devices = mockData as DeviceCardData[];

export function DeviceList() {
  return devices.map((device) => (
    <DeviceCard key={`${device.headline}-${device.subtitle}`} device={device} />
  ));
}
```

`charging_diag` is rendered as `null` when explicitly null, or as the nine snake_case
fields and their original values when present.

Run the component tests and type check from this directory:

```sh
npm ci
npm test
npm run typecheck
```
