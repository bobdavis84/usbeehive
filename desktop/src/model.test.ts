import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { cableSpeed, fixtureSnapshot, idOf, issue, relatedUsb, speed, watts, type Device } from './model';
const fixture = JSON.parse(readFileSync(new URL('../../mock-data-dbus.json', import.meta.url), 'utf8')) as Device[];
test('sanitized capture has stable topology IDs and distinct source', () => {
  const snapshot = fixtureSnapshot(fixture);
  assert.equal(snapshot.source, 'demo');
  assert.equal(snapshot.devices.length, 10);
  assert.equal(idOf(snapshot.devices[0]), 'typec:port0');
  assert.equal(new Set(snapshot.devices.map(idOf)).size, snapshot.devices.length);
});
test('capability below advertised does not automatically become a warning', () => {
  const hub = fixture.find(d => d.data_rate?.verdict === 'BelowCapability');
  assert.ok(hub);
  assert.equal(issue(hub).tone, 'notice');
  assert.equal(speed(hub.link_speed_mbps), '480 Mbps');
  assert.equal(speed(hub.data_rate!.capable_mbps), '5 Gbps');
});
test('related Type-C USB entries require explicit correlation', () => {
  const port = fixture.find(d => d.category === 'TypeCPort' && d.status !== 'Empty');
  assert.ok(port);
  assert.equal(relatedUsb(port, fixture), undefined);
});
test('power conversion only formats a supplied ceiling', () => { assert.equal(watts(65000), '65 W'); });
test('reported cable ratings and a warning are interpreted conservatively', () => {
  const device: Device = { ...fixture[1], cable: { max_watts: 100, current_rating: 'FiveAmp', speed: 'Usb32Gen2', trust: { trust_level: 'Inconclusive' } }, charging_diag: { summary: 'Cable may limit this contract', detail: 'The controller reports a mismatch.', is_warning: true, status: 'Warning', confidence: 'Medium', recommended_action: 'Try a different cable', bottleneck: 'CableLimit', root_cause: 'Cable' } };
  assert.equal(issue(device).tone, 'alert');
  assert.equal(cableSpeed(device.cable?.speed), 'USB 3.2 Gen 2');
  assert.equal(cableSpeed(null), 'Not reported');
});
