//! usbeehive CLI entry point.

use std::io::{self, Write};

use clap::Parser;

use usbeehive::{DeviceManager, Sysfs};

mod filter;
mod output;
mod report;
mod severity;

use output::{print_json, print_text, print_tree};

#[derive(Parser, Debug)]
#[command(
    name = "usbeehive",
    version,
    about = "Tells you what each USB cable / device on Linux can actually do.",
    long_about = "usbeehive — shows what each USB cable / device can do.\n\
                  Port of WhatCable (macOS) by Darryl Morley."
)]
struct Cli {
    /// Render alternate-mode, Billboard, and USB4/Thunderbolt facts and caveats.
    #[arg(long, conflicts_with_all = ["history", "watch", "summary", "format", "list", "tree"])]
    altmodes: bool,
    /// Emit timestamped transitions instead of clearing the screen (NDJSON with --json).
    #[arg(long, conflicts_with_all = ["summary", "format", "tree", "list"])]
    history: bool,
    /// Stop history capture after N samples (otherwise stream until interrupted).
    #[arg(long, requires = "history", value_parser = clap::value_parser!(u64).range(1..))]
    samples: Option<u64>,
    /// Maximum snapshots retained in memory.
    #[arg(long, requires = "history", default_value = "100", value_parser = clap::value_parser!(u64).range(1..=10000))]
    history_capacity: u64,
    /// Polling interval for bounded history capture, milliseconds.
    #[arg(long, requires = "history", default_value = "500", value_parser = clap::value_parser!(u64).range(1..))]
    interval_ms: u64,
    /// Export CSV, structured JSON, or a formatted report.
    #[arg(long, value_enum, conflicts_with_all = ["json", "summary", "watch", "tree", "list"])]
    format: Option<report::Format>,
    #[command(subcommand)]
    command: Option<Command>,
    /// Save the full, unfiltered snapshot as JSON.
    #[arg(long, conflicts_with_all = ["watch", "history"])]
    save_snapshot: Option<std::path::PathBuf>,
    /// Aggregate counts, issues, top problems, and per-port overview.
    #[arg(long)]
    summary: bool,
    #[command(flatten)]
    filters: filter::FilterArgs,
    /// Structured JSON output.
    #[arg(long, global = true)]
    json: bool,

    /// Render a flat list with full per-device details (default is a topology tree).
    #[arg(long, conflicts_with = "tree")]
    list: bool,

    /// Render the bus topology as a tree (default).
    #[arg(long, conflicts_with = "list")]
    tree: bool,

    /// Stream updates as devices change (requires the `watch` feature).
    #[arg(long)]
    watch: bool,

    /// Include raw sysfs attributes in the output.
    #[arg(long)]
    raw: bool,

    /// Override the sysfs root (default: /sys). Useful for fixture-based testing.
    #[arg(long, value_name = "PATH")]
    sysfs_root: Option<std::path::PathBuf>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Compare A with B (omit B, or use 'live', for live state).
    Diff {
        a: std::path::PathBuf,
        b: Option<std::path::PathBuf>,
    },
}

fn read_snapshot(path: &std::path::Path) -> io::Result<usbeehive::Snapshot> {
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path)?).map_err(io::Error::other)?;
    if !value.is_object() || value.get("summaries").is_none() {
        return Err(io::Error::other(
            "expected a snapshot object with summaries",
        ));
    }
    if ["usbDevices", "typecPorts", "pdPorts", "thunderboltRouters"]
        .iter()
        .any(|k| value.get(k).is_some())
    {
        return Err(io::Error::other(
            "unsupported camelCase snapshot; capture with --save-snapshot",
        ));
    }
    let snapshot: usbeehive::Snapshot = serde_json::from_value(value).map_err(io::Error::other)?;
    if snapshot.summaries.is_empty()
        && (!snapshot.usb_devices.is_empty()
            || !snapshot.typec_ports.is_empty()
            || !snapshot.pd_ports.is_empty()
            || !snapshot.thunderbolt_routers.is_empty())
    {
        return Err(io::Error::other(
            "raw-only snapshots cannot be compared; capture with --save-snapshot",
        ));
    }
    Ok(snapshot)
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    cli.filters.validate().map_err(io::Error::other)?;
    if cli.watch && !cli.history && (cli.summary || cli.filters.is_active()) {
        return Err(io::Error::other(
            "use --watch --history with filters; summary is a single-capture view",
        ));
    }
    if cli.altmodes && cli.filters.is_active() {
        return Err(io::Error::other(
            "--altmodes cannot be combined with device filters",
        ));
    }
    if cli.command.is_some() && (cli.format.is_some() || cli.raw || cli.tree || cli.list) {
        return Err(io::Error::other("diff supports only text or --json"));
    }
    let sysfs = match cli.sysfs_root.as_ref() {
        Some(p) => Sysfs::with_root(p),
        None => Sysfs::linux(),
    };
    let mut mgr = DeviceManager::with_sysfs(sysfs);
    if let Some(Command::Diff { a, b }) = &cli.command {
        if cli.watch
            || cli.history
            || cli.altmodes
            || cli.summary
            || cli.filters.is_active()
            || cli.save_snapshot.is_some()
        {
            return Err(io::Error::other(
                "diff conflicts with watch, history, altmodes, summary, filters and snapshot save",
            ));
        }
        let before = read_snapshot(a)?;
        let after = match b {
            Some(p) if p != std::path::Path::new("live") => read_snapshot(p)?,
            _ => {
                mgr.refresh();
                mgr.snapshot().clone()
            }
        };
        let diff = usbeehive::compare::compare(&before, &after);
        let stdout = io::stdout();
        let mut out = stdout.lock();
        return if cli.json {
            writeln!(
                out,
                "{}",
                serde_json::to_string_pretty(&diff).map_err(io::Error::other)?
            )
        } else {
            write!(out, "{diff}")
        };
    }

    let use_list = cli.list;
    if cli.history {
        return run_history(&mut mgr, &cli);
    }
    if cli.watch {
        #[cfg(feature = "watch")]
        return run_watch(&mut mgr, cli.json, use_list, cli.raw);
        #[cfg(not(feature = "watch"))]
        {
            eprintln!("--watch is not available: built without the `watch` feature.");
            std::process::exit(2);
        }
    }
    mgr.refresh();
    let stdout = io::stdout();
    if let Some(path) = &cli.save_snapshot {
        let bytes = serde_json::to_vec_pretty(mgr.snapshot()).map_err(io::Error::other)?;
        std::fs::write(path, bytes)?;
    }
    let mut out = stdout.lock();
    if cli.altmodes {
        let report = usbeehive::altmode::analyze(mgr.snapshot());
        return if cli.json {
            writeln!(
                out,
                "{}",
                serde_json::to_string_pretty(&report).map_err(io::Error::other)?
            )
        } else {
            write!(out, "{}", report.to_text())
        };
    }
    if cli.filters.is_active() {
        let devices = filter::apply(&cli.filters, mgr.devices());
        if let Some(format) = cli.format {
            return report::export(&mut out, &devices, format, &cli.filters, cli.raw);
        }
        return if cli.summary {
            report::print_summary(&mut out, &devices, cli.json)
        } else if cli.json {
            output::print_json_devices(&mut out, &devices, cli.raw)
        } else {
            output::print_text_iter(&mut out, &devices, cli.raw)
        };
    }
    if let Some(format) = cli.format {
        return report::export(&mut out, mgr.devices(), format, &cli.filters, cli.raw);
    }
    if cli.summary {
        return report::print_summary(&mut out, mgr.devices(), cli.json);
    }
    if cli.json {
        print_json(&mut out, &mgr, cli.raw)
    } else if use_list {
        print_text(&mut out, &mgr, cli.raw)
    } else {
        print_tree(&mut out, &mgr)
    }
}

fn run_history(mgr: &mut DeviceManager, cli: &Cli) -> io::Result<()> {
    let mut history =
        usbeehive::history::SnapshotHistory::with_capacity(cli.history_capacity as usize);
    let mut count = 0u64;
    let mut capture = || -> io::Result<()> {
        mgr.refresh();
        let mut snapshot = mgr.snapshot().clone();
        snapshot.summaries = filter::apply(&cli.filters, &snapshot.summaries);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_millis() as u64;
        history.push(snapshot, timestamp);
        let entry = history
            .latest()
            .ok_or_else(|| io::Error::other("missing history entry"))?;
        let stdout = io::stdout();
        let mut out = stdout.lock();
        if cli.json {
            writeln!(
                out,
                "{}",
                serde_json::json!({"sequence":entry.sequence,"timestampMs":timestamp,"baseline":count==0,"transitions":entry.transitions})
            )?;
        } else {
            writeln!(
                out,
                "History #{} at {} ms{}",
                entry.sequence,
                timestamp,
                if count == 0 { " (baseline)" } else { "" }
            )?;
            for transition in &entry.transitions {
                writeln!(out, "{transition}")?;
            }
        }
        count += 1;
        out.flush()
    };
    if let Some(samples) = cli.samples {
        for n in 0..samples {
            if n > 0 {
                std::thread::sleep(std::time::Duration::from_millis(cli.interval_ms));
            }
            capture()?;
        }
        return Ok(());
    }
    #[cfg(feature = "watch")]
    {
        usbeehive::watch::run_loop(std::time::Duration::from_millis(cli.interval_ms), |_| {
            capture()
        })
    }
    #[cfg(not(feature = "watch"))]
    {
        Err(io::Error::other(
            "unbounded history requires the watch feature; use --samples",
        ))
    }
}

#[cfg(feature = "watch")]
fn run_watch(
    mgr: &mut DeviceManager,
    use_json: bool,
    use_list: bool,
    show_raw: bool,
) -> io::Result<()> {
    use std::time::Duration;
    use usbeehive::watch::{run_loop, RefreshReason};

    run_loop(Duration::from_millis(500), |reason| {
        mgr.refresh();
        let stdout = io::stdout();
        let mut out = stdout.lock();
        if !use_json {
            // Clear screen + home cursor between renders so the latest snapshot
            // is always visible at the top of the terminal.
            if matches!(reason, RefreshReason::Hotplug | RefreshReason::Initial) {
                write!(out, "\x1b[2J\x1b[H")?;
            }
            if use_list {
                print_text(&mut out, mgr, show_raw)?;
            } else {
                print_tree(&mut out, mgr)?;
            }
        } else {
            print_json(&mut out, mgr, show_raw)?;
        }
        out.flush()
    })
}
