//! Binary-local aggregate reports.
use crate::severity::{severity_of, Severity};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{self, Write};
use usbeehive::summary::{Category, DeviceSummary};

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
    Report,
}

pub fn print_summary(
    w: &mut impl Write,
    devices: &[DeviceSummary],
    json_output: bool,
) -> io::Result<()> {
    let value = summary(devices);
    if json_output {
        return writeln!(w, "{value}");
    }
    writeln!(w, "Devices: {}\nCounts by type:", devices.len())?;
    for (kind, count) in value["countsByType"].as_object().into_iter().flatten() {
        writeln!(w, "  {kind}: {count}")?;
    }
    writeln!(
        w,
        "PD bottlenecks: {}\nCable issues: {}\nTop problems:",
        value["pdBottlenecks"], value["cableIssues"]
    )?;
    for problem in value["topProblems"].as_array().into_iter().flatten() {
        writeln!(
            w,
            "  {}: {} ({})",
            problem["id"], problem["headline"], problem["severity"]
        )?;
    }
    writeln!(w, "Per-port overview:")?;
    for d in devices.iter().filter(|d| d.category == Category::TypeCPort) {
        writeln!(
            w,
            "  {}: {} — {:?}; {} mW",
            d.id(),
            d.headline,
            d.status,
            d.power.power_in_mw
        )?;
    }
    Ok(())
}

pub fn export(
    w: &mut impl Write,
    devices: &[DeviceSummary],
    format: Format,
    filters: &crate::filter::FilterArgs,
    raw: bool,
) -> io::Result<()> {
    match format {
        Format::Json => crate::output::print_json_devices(w, devices, raw),
        Format::Csv => {
            writeln!(
                w,
                "id,headline,severity,linkSpeedMbps,vendorId,productId,status,recommendation"
            )?;
            for d in devices {
                let fields = [
                    d.id(),
                    d.headline.clone(),
                    severity_of(d).label().into(),
                    d.link_speed_mbps.to_string(),
                    format!("{:04x}", d.vendor_id),
                    format!("{:04x}", d.product_id),
                    format!("{:?}", d.status),
                    d.charging_diag
                        .as_ref()
                        .map(|x| x.recommended_action.clone())
                        .unwrap_or_default(),
                ];
                writeln!(
                    w,
                    "{}",
                    fields
                        .iter()
                        .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
                        .collect::<Vec<_>>()
                        .join(",")
                )?;
            }
            Ok(())
        }
        Format::Report => {
            writeln!(
                w,
                "USBeehive report\nFilters: {}\nSummary:\n{}",
                filters.describe().join(", "),
                serde_json::to_string_pretty(&summary(devices)).map_err(io::Error::other)?
            )?;
            crate::output::print_text_iter(w, devices, raw)
        }
    }
}

pub fn summary(devices: &[DeviceSummary]) -> Value {
    let mut counts = BTreeMap::<String, usize>::new();
    let mut bottlenecks = BTreeMap::<String, usize>::new();
    let mut severities = BTreeMap::new();
    for s in Severity::ALL {
        severities.insert(s.label(), 0usize);
    }
    for d in devices {
        *counts
            .entry(if d.category == Category::TypeCPort {
                "TypeCPort".into()
            } else {
                format!("{:?}", d.device_class)
            })
            .or_default() += 1;
        *severities.entry(severity_of(d).label()).or_default() += 1;
        if let Some(diag) = &d.charging_diag {
            *bottlenecks
                .entry(format!("{:?}", diag.bottleneck))
                .or_default() += 1;
        }
    }
    let mut problems: Vec<_> = devices
        .iter()
        .filter(|d| severity_of(d) >= Severity::Warning)
        .collect();
    problems.sort_by_key(|d| (std::cmp::Reverse(severity_of(d)), d.id()));
    json!({"total": devices.len(), "countsByType": counts, "severityCounts": severities,
        "pdBottlenecks": bottlenecks, "cableIssues": devices.iter().filter(|d| d.cable.as_ref().is_some_and(|c| c.trust.is_suspicious())).count(),
        "topProblems": problems.into_iter().take(10).map(|d| json!({"id":d.id(),"headline":d.headline,"severity":severity_of(d),"reasons":crate::severity::assess(d).reasons})).collect::<Vec<_>>(),
        "ports": devices.iter().filter(|d| d.category == Category::TypeCPort).map(|d| crate::output::device_json(d,false)).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_quotes_embedded_commas_quotes_and_newlines() {
        let mut d = DeviceSummary::from_usb_device(&usbeehive::usb::UsbDevice::default());
        d.headline = "A, \"quoted\"\nname".into();
        let mut out = Vec::new();
        export(&mut out, &[d], Format::Csv, &Default::default(), false).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\"A, \"\"quoted\"\"\nname\""));
    }
    #[test]
    fn report_propagates_write_errors() {
        struct Broken;
        impl std::io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("broken"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        for format in [Format::Csv, Format::Json, Format::Report] {
            assert!(export(&mut Broken, &[], format, &Default::default(), false).is_err());
        }
    }
}
