import { useEffect, useMemo, useState } from "react";
import fixture from "../../mock-data-dbus.json";
import { cableSpeed, connectionPath, fixtureSnapshot, idOf, issue, meaningfulProps, parentOf, props, relatedUsb, speed, watts, type Device, type Snapshot } from "./model";

const demo = !('__TAURI_INTERNALS__' in window) || new URLSearchParams(location.search).has('demo');
const initial = fixtureSnapshot(fixture as Device[]);
type View = 'connections' | 'changes' | 'about';
type Filter = 'all' | 'attention' | 'typec' | 'usb';

function Mark() { return <svg aria-hidden="true" className="mark" viewBox="0 0 38 38" fill="none"><path d="M19 4v23M10 14v10m18-15v15M10 24h18M7 28h24v6H7z" stroke="currentColor" strokeWidth="2.4" strokeLinecap="square"/><circle cx="19" cy="5" r="2" fill="currentColor"/><circle cx="28" cy="9" r="2" fill="currentColor"/></svg>; }
function Pill({ tone, children }: { tone: string; children: React.ReactNode }) { return <span className={`pill pill--${tone}`}>{children}</span>; }
function DeviceName({ device }: { device: Device }) { return <><strong>{device.headline}</strong><span className="secondary">{device.subtitle || device.category}</span></>; }
function Fact({ label, value, note }: { label: string; value: string; note?: string }) { return <div className="fact"><span>{label}</span><strong className="num">{value}</strong>{note && <small>{note}</small>}</div>; }
function Pair({ label, value }: { label: string; value: string }) { return <div className="pair"><dt>{label}</dt><dd>{value || 'Not reported'}</dd></div>; }
function stateLabel(d: Device) { return d.category === 'TypeCPort' ? (d.status === 'Empty' ? 'Available port' : 'Connected port') : d.category === 'Hub' ? 'USB hub' : d.device_class.replace(/([a-z])([A-Z])/g, '$1 $2'); }
function changeLabel(kind: string): string {
  return ({ connected: 'Connected', disconnected: 'Disconnected', changed: 'Connection changed', power_warning: 'Charging limitation detected', power_restored: 'Charging limitation cleared', data_warning: 'Data link degraded', data_restored: 'Data link restored' } as Record<string, string>)[kind] || 'Connection changed';
}
function Detail({ device, all }: { device: Device; all: Device[] }) {
  const signal = issue(device), path = connectionPath(device, all), related = device.typec_port ? relatedUsb(device, all) : undefined;
  const cap = device.data_rate?.capable_mbps || Number(props(device).get('usb_capable_speed_mbps')) || 0;
  const propRows = meaningfulProps(device);

  const isUsb = device.category !== 'TypeCPort';
  const busPowerAlloc = props(device).get('usb.bus_power_allocated');
  const powerSource = props(device).get('usb.power_source');
  const hostBattery = props(device).get('host.battery');
  const hostMains = props(device).get('host.mains_online');

  let reqLabel = "Requested operating point";
  let reqVal = device.power.power_in_mw ? `Up to ${watts(device.power.power_in_mw)}` : 'Not reported';
  let reqNote = "Not measured consumption";
  if (isUsb && busPowerAlloc) {
    reqLabel = "Bus power allocation";
    reqVal = busPowerAlloc;
    reqNote = "Standard USB descriptor ceiling (bMaxPower)";
  } else if (!device.power.power_in_mw && hostBattery) {
    reqLabel = "Host battery telemetry";
    reqVal = hostBattery;
    reqNote = "System battery status & charge rate";
  }

  let conLabel = "Contract allows";
  let conVal = device.power.contract_mw ? `Up to ${watts(device.power.contract_mw)}` : 'Not reported';
  let conNote = "Negotiated ceiling, if available";
  if (isUsb && powerSource) {
    conLabel = "Power source";
    conVal = powerSource;
    conNote = "From configuration bmAttributes";
  } else if (!device.power.contract_mw && hostMains) {
    conLabel = "Host mains power";
    conVal = hostMains === "true" ? "AC Mains online" : "On battery";
    conNote = "System power supply telemetry";
  }

  return <section className="detail" aria-labelledby="detail-heading">
    <div className="detail-head"><div><span className="context">{stateLabel(device)}</span><h2 id="detail-heading">{device.headline}</h2><p>{device.subtitle || 'Hardware details from Linux sysfs'}</p></div><Pill tone={signal.tone}>{signal.tone === 'alert' ? 'Needs attention' : signal.tone === 'notice' ? 'Worth checking' : signal.tone === 'good' ? 'On track' : 'No finding'}</Pill></div>
    <div className="finding" data-tone={signal.tone}><span className="finding-mark"/><div><strong>{signal.text}</strong><p>{signal.detail}</p>{device.charging_diag?.recommended_action && <p className="action-copy">Next check: {device.charging_diag.recommended_action}</p>}</div></div>
    <div className="section-heading"><h3>Connection path</h3><span>Only reported relationships are linked</span></div>
    <div className="path" aria-label={`${path.origin} to ${path.end}`}><div><small>Origin</small><strong>{path.origin}</strong></div><div><small>Connection</small><strong>{path.middle}</strong></div><div><small>Endpoint</small><strong>{path.end}</strong></div></div>
    {related && <p className="relation">The USB device associated with this Type-C port is <strong>{related.headline}</strong>. Its link speed is {speed(related.link_speed_mbps)}.</p>}
    <div className="section-heading"><h3>Data and power</h3><span>Advertised limits are not measurements</span></div>
    <div className="fact-grid"><Fact label="Negotiated USB link" value={speed(device.link_speed_mbps)} note={device.link_speed_mbps ? 'Current link rate' : 'No USB link rate exposed for this entry'}/><Fact label="Advertised capability" value={speed(cap)} note={cap ? 'From device descriptors' : 'No speed capability reported'}/><Fact label={reqLabel} value={reqVal} note={reqNote}/><Fact label={conLabel} value={conVal} note={conNote}/></div>
    {device.usb_device?.interfaces && device.usb_device.interfaces.length > 0 && (
      <div className="evidence">
        <div className="section-heading"><h3>USB interfaces & endpoints</h3><span>{device.usb_device.interfaces.length} interface{device.usb_device.interfaces.length === 1 ? '' : 's'}</span></div>
        <div className="interfaces-box">
          {device.usb_device.interfaces.map(iface => (
            <div key={iface.number} className="interface-row">
              <div className="interface-title">
                <strong>Interface {iface.number}: {iface.name || 'Descriptor'}</strong>
                {iface.driver && <Pill tone="neutral">{iface.driver}</Pill>}
              </div>
              {iface.endpoints && iface.endpoints.length > 0 && (
                <div className="endpoints-flow">
                  {iface.endpoints.map(ep => (
                    <span key={ep.address} className="ep-badge">
                      0x{ep.address.toString(16).padStart(2, '0')} · {ep.ep_type} {ep.direction === 'in' ? 'In' : 'Out'} ({ep.max_packet_size}B{ep.interval && ep.interval !== '0ms' ? `, ${ep.interval}` : ''})
                    </span>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      </div>
    )}
    {device.cable && <div className="evidence"><div className="section-heading"><h3>Cable evidence</h3><span>From reported e-marker data</span></div><dl><Pair label="Type" value={device.cable.cable_type || ''}/><Pair label="Vendor" value={device.cable.vendor_name || ''}/><Pair label="Current rating" value={device.cable.current_rating === 'ThreeAmp' ? '3 A' : device.cable.current_rating === 'FiveAmp' ? '5 A' : device.cable.current_rating === 'UsbDefault' ? 'USB default' : 'Not reported'}/><Pair label="Power rating" value={device.cable.max_watts ? `Up to ${device.cable.max_watts} W` : 'Not reported'}/><Pair label="Data capability" value={cableSpeed(device.cable.speed)}/><Pair label="Evidence level" value={device.cable.trust?.trust_level || 'Inconclusive'}/></dl><p className="fineprint">A clean descriptor is not a physical safety certification. Missing identity data is inconclusive.</p></div>}
    {device.charging_diag && <div className="evidence"><div className="section-heading"><h3>Charging assessment</h3><span>{device.charging_diag.confidence} confidence</span></div><p>{device.charging_diag.detail}</p><dl><Pair label="Likely cause" value={device.charging_diag.root_cause}/><Pair label="Assessment" value={device.charging_diag.bottleneck}/></dl></div>}
    {device.power_delivery?.source_capabilities?.length ? <div className="evidence"><div className="section-heading"><h3>Charger profiles</h3><span>Advertised by source</span></div><div className="profiles">{device.power_delivery.source_capabilities.map(p => <div className="profile" key={p.index}><span>{p.is_active ? <Pill tone="good">Active contract</Pill> : `Profile ${p.index}`}</span><span className="num">{Number((p.voltage_mv / 1000).toFixed(1))} V · {Number((p.current_ma / 1000).toFixed(2))} A</span><strong className="num">Up to {watts(p.power_mw)}</strong></div>)}</div></div> : null}
    <details className="technical"><summary>Hardware evidence</summary><dl><Pair label="Category" value={device.category}/><Pair label="USB version" value={device.usb_version}/><Pair label="Driver" value={device.primary_driver}/><Pair label="Vendor / product ID" value={`${device.vendor_id.toString(16).padStart(4, '0')}:${device.product_id.toString(16).padStart(4, '0')}`}/>{device.data_rate && <Pair label="Data-rate evidence" value={`${device.data_rate.summary}. ${device.data_rate.detail}`}/ >}{propRows.map(([key, value]) => <Pair key={key} label={key} value={value}/>)}</dl><p className="fineprint">Missing data does not prove a cable is absent, faulty, or uncertified. Link speed and USB-PD values depend on what the kernel and controller expose.</p></details>
  </section>;
}
function App() {
  const [snapshot, setSnapshot] = useState<Snapshot>(demo ? initial : { devices: [], events: [], source: 'live', updated_at_ms: 0 });
  const [view, setView] = useState<View>('connections');
  const [filter, setFilter] = useState<Filter>('all');
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (demo) return;
    let mounted = true; let unlisten: (() => void) | undefined;
    Promise.all([import('@tauri-apps/api/core'), import('@tauri-apps/api/event')]).then(async ([{ invoke }, { listen }]) => {
      const stop = await listen<Snapshot>('snapshot-updated', event => { if (mounted) { setSnapshot(event.payload); setError(null); } });
      if (!mounted) { stop(); return; }
      unlisten = stop;
      try { const next = await invoke<Snapshot>('get_snapshot'); if (mounted) setSnapshot(next); }
      catch (e) { if (mounted) setError(String(e)); }
    }).catch(e => { if (mounted) setError(String(e)); });
    return () => { mounted = false; unlisten?.(); };
  }, []);
  const devices = snapshot.devices;
  const attention = devices.filter(d => issue(d).tone === 'alert').length;
  const visible = useMemo(() => devices.filter(d => {
    const match = filter === 'all' || (filter === 'attention' && issue(d).tone === 'alert') || (filter === 'typec' && d.category === 'TypeCPort') || (filter === 'usb' && d.category !== 'TypeCPort');
    return match && `${d.headline} ${d.subtitle} ${d.vendor} ${d.usb_device?.bus_port || ''}`.toLowerCase().includes(query.trim().toLowerCase());
  }), [devices, filter, query]);
  const active = devices.find(d => idOf(d) === selected) || visible.find(d => d.data_rate?.verdict === 'BelowCapability' && d.category === 'UsbDevice') || visible.find(d => d.status !== 'Empty') || visible[0];
  async function refresh() { if (demo) return; setBusy(true); try { const { invoke } = await import('@tauri-apps/api/core'); setSnapshot(await invoke<Snapshot>('refresh_snapshot')); setError(null); } catch (e) { setError(String(e)); } finally { setBusy(false); } }
  async function exportCapture() {
    const safe = { captured_at_ms: snapshot.updated_at_ms, source: snapshot.source, devices: devices.map(d => ({ id: idOf(d), category: d.category, headline: d.headline, subtitle: d.subtitle, status: d.status, link_speed_mbps: d.link_speed_mbps, power: d.power, charging_diag: d.charging_diag, data_rate: d.data_rate })) };
    const contents = JSON.stringify(safe, null, 2);
    try {
      if (!demo) {
        const [{ save }, { writeTextFile }] = await Promise.all([import('@tauri-apps/plugin-dialog'), import('@tauri-apps/plugin-fs')]);
        const path = await save({ defaultPath: 'usbeehive-capture.json', filters: [{ name: 'JSON', extensions: ['json'] }] });
        if (path) await writeTextFile(path, contents);
      } else {
        const url = URL.createObjectURL(new Blob([contents], { type: 'application/json' }));
        const a = document.createElement('a'); a.href = url; a.download = 'usbeehive-capture.json'; a.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
      }
      setError(null);
    } catch (e) { setError(`Export failed: ${String(e)}`); }
  }
  return <div className="shell"><aside className="sidebar"><div className="brand"><Mark/><span>USBeehive<small>Connection workbench</small></span></div><div className="scope">This computer <span className="scope-dot" data-demo={demo}/></div><nav aria-label="Main navigation"><button className={view === 'connections' ? 'active' : ''} onClick={() => setView('connections')}>Connections <span className="nav-count num">{devices.length}</span></button><button className={view === 'changes' ? 'active' : ''} onClick={() => setView('changes')}>Changes {snapshot.events.length > 0 && <span className="nav-count num">{snapshot.events.length}</span>}</button><button className={view === 'about' ? 'active' : ''} onClick={() => setView('about')}>How to read this</button></nav><div className="sidebar-bottom"><span className="live-indicator" data-demo={demo}/><span>{demo ? 'Sanitized capture' : 'Local hardware'}</span><small>{demo ? 'Preview · not live' : 'Linux sysfs · local only'}</small></div></aside>
  <div className="main-area"><header className="topbar"><div className="mobile-brand"><Mark/> USBeehive</div><span className="crumb">This computer <span aria-hidden="true">/</span> {view === 'connections' ? 'Connections' : view === 'changes' ? 'Changes' : 'How to read this'}</span><div className="top-actions"><span className="source-label">{demo ? 'Sanitized capture · demo' : 'Live from this computer'}</span><button className="button button-secondary" onClick={refresh} disabled={demo || busy}>{busy ? 'Refreshing…' : 'Refresh'}</button></div></header>
  <nav className="mobile-nav" aria-label="Mobile navigation"><button className={view === 'connections' ? 'active' : ''} onClick={() => setView('connections')}>Connections</button><button className={view === 'changes' ? 'active' : ''} onClick={() => setView('changes')}>Changes</button><button className={view === 'about' ? 'active' : ''} onClick={() => setView('about')}>How to read this</button></nav><main className="sheet">{view === 'connections' ? <><div className="page-head"><div><h1>Connections</h1><p>{attention ? `${attention} connection${attention === 1 ? '' : 's'} need attention.` : 'See what your USB devices and ports actually report.'} {demo && 'This is a sanitized capture, not live hardware.'}</p></div><button className="button button-plain" onClick={exportCapture}>Export summary ↗</button></div>{error && <div role="alert" className="error">{error} <button onClick={refresh}>Retry refresh</button></div>}
    <div className="overview"><div><span>Connections in view</span><strong className="num">{devices.length}</strong></div><div><span>USB-C ports</span><strong className="num">{devices.filter(d => d.category === 'TypeCPort').length}</strong></div><div><span>Needs attention</span><strong className="num">{attention}</strong></div><div><span>Data source</span><strong className="small-value">{demo ? 'Captured snapshot' : 'Linux sysfs'}</strong></div></div>
    <div className={`workspace ${selected ? 'mobile-detail-open' : ''}`}><section className="device-list" aria-labelledby="list-heading"><div className="list-heading"><div><h2 id="list-heading">Devices and ports</h2><p>{visible.length} shown of {devices.length}</p></div></div><div className="tabs" role="group" aria-label="Connection filter">{([['all','All'],['attention','Attention'],['typec','USB-C ports'],['usb','USB devices']] as [Filter,string][]).map(([value,label]) => <button key={value} className={filter === value ? 'selected' : ''} onClick={() => { setFilter(value); setSelected(null); }}>{label}</button>)}</div><label className="search-label" htmlFor="device-search">Search connections</label><input id="device-search" className="search" type="search" value={query} placeholder="Name, vendor or bus address" onChange={e => {setQuery(e.target.value);setSelected(null);}}/><div className="rows">{visible.length ? visible.map(d => { const signal = issue(d); return <button key={idOf(d)} className={`device-row ${active && idOf(active) === idOf(d) ? 'is-selected' : ''}`} onClick={() => { setSelected(idOf(d)); window.scrollTo(0, 0); }} aria-current={active && idOf(active) === idOf(d) ? 'true' : undefined}><span className="row-main"><DeviceName device={d}/></span><span className="row-meta"><span className={`row-signal signal--${signal.tone}`}>{signal.tone === 'alert' ? 'Attention' : signal.tone === 'notice' ? 'Below capability' : stateLabel(d)}</span><span className="num">{d.link_speed_mbps ? speed(d.link_speed_mbps) : d.category === 'TypeCPort' ? d.status : '—'}</span></span></button>}) : <div className="empty">{!demo && !snapshot.updated_at_ms && !error ? 'Reading local hardware…' : 'No connections match this view. Try another filter or search.'}</div>}</div></section><button className="mobile-back" onClick={() => { setSelected(null); window.scrollTo(0, 0); }}>← All connections</button>{active ? <Detail key={idOf(active)} device={active} all={devices}/> : <section className="detail empty-detail"><h2>No connection selected</h2><p>{!demo && !snapshot.updated_at_ms && !error ? 'Reading local hardware…' : 'Choose a connection to inspect its evidence.'}</p></section>}</div>
  </> : view === 'changes' ? <div className="simple-view"><h1>Changes</h1><p>Connection transitions recorded while this app is open. They are not persistent history.</p>{snapshot.events.length ? <div className="change-list">{snapshot.events.slice().reverse().map((event, i) => <div className="change-row" key={`${event.device_id}-${event.at_ms}-${i}`}><strong>{changeLabel(event.kind)}</strong><span>{devices.find(d => idOf(d) === event.device_id)?.headline || event.device_id}</span><time className="num">{event.at_ms ? new Date(event.at_ms).toLocaleTimeString() : 'This session'}</time></div>)}</div> : <div className="empty large"><strong>No changes recorded yet</strong><p>{demo ? 'The sanitized snapshot does not include a live event history.' : 'Plug or unplug a device to see changes here. Some controllers do not emit every state change.'}</p></div>}</div> : <div className="simple-view read-view"><h1>How to read this</h1><p>USBeehive reports what Linux exposes about your connections. It does not measure current with a meter or certify a cable.</p><h2>Data speed</h2><p>Negotiated speed is the current USB link. Advertised capability comes from the device’s descriptors when available. A lower link does not by itself prove a faulty cable; a hub, port, or intentional connection can also limit it.</p><h2>Power</h2><p>“Requested operating point” describes a negotiated request or ceiling, not live watts consumed. “Contract allows” is the active power-delivery profile’s ceiling. Missing PD or e-marker information means it was not reported, not that the cable is unsafe.</p><h2>Privacy</h2><p>The app reads local sysfs only. Export summary excludes serial numbers, raw attributes and filesystem paths; review device names before sharing a capture.</p></div>}</main></div></div>;
}
export default App;
