#!/usr/bin/env python3
"""Run the project's offline feature matrix without masking subprocess failures."""

import argparse
import datetime
import json
import os
from pathlib import Path
import re
import subprocess
import sys


GATES = [
    ("fmt", ["fmt", "--all", "--check"], False),
    ("clippy-default", ["clippy", "--offline", "--all-targets", "--", "-D", "warnings"], False),
    ("clippy-all", ["clippy", "--offline", "--all-targets", "--all-features", "--", "-D", "warnings"], False),
    ("test-default", ["test", "--offline"], False),
    ("test-core", ["test", "--offline", "--no-default-features"], False),
    ("test-cli-only", ["test", "--offline", "--no-default-features", "--features", "cli"], False),
    ("test-dbus", ["test", "--offline", "--no-default-features", "--features", "dbus"], False),
    ("test-all", ["test", "--offline", "--all-features"], False),
    ("doc-default", ["doc", "--offline", "--no-deps"], True),
    ("doc-all", ["doc", "--offline", "--no-deps", "--all-features"], True),
    ("release-dbus", ["build", "--offline", "--release", "--no-default-features", "--features", "dbus"], False),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[3])
    parser.add_argument("--logs", type=Path, required=True)
    parser.add_argument("--only", nargs="*", choices=[name for name, _, _ in GATES])
    args = parser.parse_args()
    args.logs.mkdir(parents=True, exist_ok=True)
    results = []
    for name, options, docs in GATES:
        if args.only and name not in args.only:
            continue
        command = ["cargo", *options]
        env = os.environ.copy()
        if docs:
            env["RUSTDOCFLAGS"] = "-D warnings"
        start = datetime.datetime.now(datetime.timezone.utc)
        log_path = args.logs / f"{name}.log"
        print(f"RUN {name}: {' '.join(command)}", flush=True)
        with log_path.open("w") as log:
            process = subprocess.run(command, cwd=args.root, env=env, stdout=log, stderr=subprocess.STDOUT, check=False)
        text = log_path.read_text()
        groups = re.findall(r"test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored", text)
        result = {
            "gate": name,
            "command": command,
            "environment_overrides": {"RUSTDOCFLAGS": "-D warnings"} if docs else {},
            "started_at": start.isoformat(),
            "duration_seconds": (datetime.datetime.now(datetime.timezone.utc) - start).total_seconds(),
            "exit_code": process.returncode,
            "passed": sum(int(group[1]) for group in groups),
            "failed": sum(int(group[2]) for group in groups),
            "ignored": sum(int(group[3]) for group in groups),
            "log": str(log_path.resolve()),
        }
        results.append(result)
        (args.logs / "results.json").write_text(json.dumps(results, indent=2) + "\n")
        print(json.dumps(result), flush=True)
        if process.returncode:
            print(text[-12000:], flush=True)
    return int(any(result["exit_code"] != 0 for result in results))


if __name__ == "__main__":
    sys.exit(main())
