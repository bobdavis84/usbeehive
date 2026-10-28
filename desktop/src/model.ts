export interface Endpoint {
  address: number;
  direction: string;
  ep_type: string;
  max_packet_size: number;
  interval: string;
}

export interface Interface {
  number: number;
  class_code: number;
  sub_class: number;
  protocol: number;
  driver: string;
  name?: string;
  endpoints?: Endpoint[];
}

export interface Device {
  id?: string;
  category: string;
  device_class: string;
  status: string;
  headline: string;
  subtitle: string;
  vendor: string;
  product: string;
  vendor_id: number;
  product_id: number;
  primary_driver: string;
  properties: [string, string][];
  link_speed_mbps: number;
  usb_version: string;
  power: { power_in_mw: number; power_out_mw: number; contract_mw: number; power_role: string };
  charging_diag: null | { summary: string; detail: string; is_warning: boolean; status: string; confidence: string; recommended_action: string; bottleneck: string; root_cause: string };
  data_rate: null | { verdict: string; summary: string; detail: string; is_warning: boolean; capable_mbps: number; functional_floor_mbps: number };
  usb_device: null | { bus_port: string; serial?: string; parent_bus_port?: string; interfaces?: Interface[]; port_location?: string };
  typec_port: null | { port_name: string; port_number: number; partner?: { usb_name?: string } | null; port_location?: string };
  cable: null | { max_watts?: number; vendor_name?: string; cable_type?: string; speed?: string | null; current_rating?: string | null; trust?: { trust_level?: string; notes?: string[] } };
  power_delivery: null | { source_capabilities: { index: number; voltage_mv: number; max_voltage_mv: number; current_ma: number; power_mw: number; is_active: boolean; type?: string }[] };
}
export type Change = { device_id: string; kind: string; at_ms: number };
export type Snapshot = { devices: Device[]; events: Change[]; source: string; updated_at_ms: number };
export function idOf(d: Device): string {
  return d.id || (d.typec_port ? `typec:${d.typec_port.port_name}` : d.usb_device ? `usb:${d.usb_device.bus_port}` : `${d.category}:${d.headline}`);
}
export function props(d: Device): Map<string, string> { return new Map(d.properties); }
export function speed(mbps: number): string {
  if (!mbps) return "Not reported";
  if (mbps >= 1000) return `${Number((mbps / 1000).toFixed(1))} Gbps`;
  return `${mbps} Mbps`;
}
export function watts(mw: number): string { return `${Number((mw / 1000).toFixed(1))} W`; }
export function cableSpeed(value?: string | null): string {
  if (!value) return 'Not reported';
  return ({ Usb20: 'USB 2.0', Usb32Gen1: 'USB 3.2 Gen 1', Usb32Gen2: 'USB 3.2 Gen 2', Usb4Gen3: 'USB4 Gen 3', Usb4Gen4: 'USB4 Gen 4' } as Record<string, string>)[value] || value;
}
export function issue(d: Device): { tone: "alert" | "notice" | "good" | "neutral"; text: string; detail: string } {
  if (d.charging_diag?.is_warning) return { tone: "alert", text: d.charging_diag.summary, detail: d.charging_diag.detail };
  if (d.data_rate?.is_warning) return { tone: "alert", text: d.data_rate.summary, detail: d.data_rate.detail };
  if (d.charging_diag) return { tone: "good", text: d.charging_diag.summary, detail: d.charging_diag.detail };
  if (d.data_rate?.verdict === "BelowCapability") return { tone: "notice", text: d.data_rate.summary, detail: d.data_rate.detail };
  if (d.data_rate?.verdict === "AtCapability") return { tone: "good", text: d.data_rate.summary, detail: d.data_rate.detail };
  return { tone: "neutral", text: d.status === "Empty" ? "No device attached" : "No limitation reported", detail: "Available controller data may not describe the cable or negotiated power." };
}
export function relatedUsb(port: Device, all: Device[]): Device | undefined {
  const id = props(port).get("usb_device");
  return id ? all.find(d => idOf(d) === id) : undefined;
}
export function relatedPort(usbDev: Device, all: Device[]): Device | undefined {
  const id = idOf(usbDev);
  return all.find(p => p.category === 'TypeCPort' && props(p).get('usb_device') === id);
}
export function parentOf(d: Device, all: Device[]): Device | undefined {
  const bus = d.usb_device?.bus_port;
  if (!bus || !bus.includes(".")) return undefined;
  const parent = bus.slice(0, bus.lastIndexOf("."));
  return all.find(x => x.usb_device?.bus_port === parent);
}
export function connectionPath(d: Device, all: Device[]) {
  const parent = parentOf(d, all);
  const linkedUsb = d.category === 'TypeCPort' ? relatedUsb(d, all) : undefined;
  const linkedPort = d.category !== 'TypeCPort' ? relatedPort(d, all) : undefined;
  const portLoc = props(d).get('port.location') || (linkedPort ? props(linkedPort).get('port.location') : undefined);
  const locSuffix = portLoc ? ` (${portLoc})` : '';

  const origin = d.typec_port
    ? `${d.headline}${locSuffix}`
    : linkedPort
    ? `${linkedPort.headline}${locSuffix}`
    : parent?.headline || (props(d).get('port.id') ? `${props(d).get('port.id')}${locSuffix}` : 'USB bus');

  const middle = d.typec_port
    ? (d.cable ? 'Cable e-marker available' : linkedUsb ? `${speed(linkedUsb.link_speed_mbps)} link` : 'Cable not reported')
    : parent
    ? 'Through hub'
    : linkedPort
    ? `${speed(d.link_speed_mbps)} Type-C link`
    : 'USB link';

  const end = d.typec_port
    ? (linkedUsb ? linkedUsb.headline : d.status === 'Empty' ? 'No device attached' : 'USB device not correlated')
    : d.headline;

  return { origin, middle, end, location: portLoc };
}
export function meaningfulProps(d: Device): [string, string][] {
  const keys = [
    "port.location",
    "port.id",
    "port.peer_id",
    "port.peer_state",
    "port.connect_type",
    "usb.power_source",
    "usb.bus_power_allocated",
    "host.mains_online",
    "host.battery",
    "kernel.quirks",
    "hub.ports_total",
    "hub.ports_used",
    "hub.power_budget_ma",
    "hub.power_committed_ma",
    "pd_contract",
    "transport.dp_altmode",
    "transport.tb",
  ];
  const map = props(d);
  const out: [string, string][] = [];
  for (const k of keys) {
    if (map.has(k)) out.push([k, map.get(k)!]);
  }
  for (const [k, v] of d.properties) {
    if (k.startsWith("usb.interface_")) {
      out.push([k, v]);
    }
  }
  return out;
}
export function fixtureSnapshot(devices: Device[]): Snapshot { return { devices, events: [], source: "demo", updated_at_ms: 0 }; }
