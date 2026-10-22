import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import DeviceCard, { type DeviceCardData } from "./DeviceCard.tsx";

const snapshot = JSON.parse(
  readFileSync(new URL("../mock-data-dbus.json", import.meta.url), "utf8"),
) as DeviceCardData[];

test("renders headline, subtitle, and null charging_diag from the D-Bus fixture", () => {
  const device = snapshot[0];
  assert.ok(device);
  assert.equal(device.charging_diag, null);

  const html = renderToStaticMarkup(createElement(DeviceCard, { device }));
  assert.ok(html.includes(`>${device.headline}</h2>`));
  assert.ok(html.includes(`<p class="device-card__subtitle">${device.subtitle}</p>`));
  assert.ok(html.includes(`<h3>charging_diag</h3>`));
  assert.ok(html.includes(`<dd>null</dd>`));
});

test("renders every charging_diag field name and value without renaming", () => {
  const device: DeviceCardData = {
    headline: "Fixture device",
    subtitle: "Fixture subtitle",
    charging_diag: {
      bottleneck: "CableLimit",
      summary: "Cable rating is at or below reported sink demand",
      detail: "Sink requests up to 80W, but the cable is rated for 60W.",
      is_warning: true,
      status: "Warning",
      root_cause: "Cable",
      confidence: "High",
      recommended_action: "Use an 80W cable.",
      rule_id: "CABLE_LIMIT",
    },
  };

  const html = renderToStaticMarkup(createElement(DeviceCard, { device }));
  const diagnostic = device.charging_diag;
  if (diagnostic === null) throw new Error("fixture diagnostic unexpectedly null");
  for (const [field, value] of Object.entries(diagnostic)) {
    assert.ok(html.includes(`<dt>${field}</dt>`), `missing field ${field}`);
    assert.ok(html.includes(`<dd>${String(value)}</dd>`), `missing value for ${field}`);
  }
});

test("renders false warning values rather than dropping the boolean", () => {
  const device: DeviceCardData = {
    headline: "Healthy fixture",
    subtitle: "Synthetic diagnostic",
    charging_diag: {
      bottleneck: "Fine",
      summary: "Charging at the available rate",
      detail: "",
      is_warning: false,
      status: "Healthy",
      root_cause: "None",
      confidence: "High",
      recommended_action: "",
      rule_id: "FINE",
    },
  };
  const html = renderToStaticMarkup(createElement(DeviceCard, { device }));
  assert.ok(html.includes('data-warning="false"'));
  assert.ok(html.includes('<dt>is_warning</dt><dd>false</dd>'));
  assert.ok(html.includes('<dt>recommended_action</dt><dd></dd>'));
});

test("treats device-provided markup as text", () => {
  const device: DeviceCardData = {
    headline: "<script>alert(1)</script>",
    subtitle: "<img src=x onerror=alert(1)>",
    charging_diag: null,
  };
  const html = renderToStaticMarkup(createElement(DeviceCard, { device }));
  assert.ok(!html.includes("<script>"));
  assert.ok(!html.includes("<img"));
  assert.ok(html.includes("&lt;script&gt;alert(1)&lt;/script&gt;"));
  assert.ok(html.includes("&lt;img src=x onerror=alert(1)&gt;"));
});
