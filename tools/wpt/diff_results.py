#!/usr/bin/env python3
import argparse
import json
from pathlib import Path


def load(path: str) -> dict:
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def by_path(payload: dict) -> dict:
    return {entry["path"]: entry for entry in payload.get("results", [])}


def main() -> int:
    parser = argparse.ArgumentParser(description="Diff two WPT result JSON files")
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--latest", required=True)
    args = parser.parse_args()

    baseline = load(args.baseline)
    latest = load(args.latest)

    base_by_path = by_path(baseline)
    latest_by_path = by_path(latest)

    regressions = []
    improvements = []

    for path, current in latest_by_path.items():
        old = base_by_path.get(path)
        if not old:
            continue

        old_outcome = old.get("outcome")
        new_outcome = current.get("outcome")

        if old_outcome in {"pass", "unexpected_pass"} and new_outcome in {
            "regression",
            "fail",
            "expected_fail",
        }:
            regressions.append(path)
        if old_outcome in {"regression", "fail", "expected_fail"} and new_outcome in {
            "pass",
            "unexpected_pass",
        }:
            improvements.append(path)

    print(f"Baseline: {Path(args.baseline)}")
    print(f"Latest:   {Path(args.latest)}")
    print(f"Regressions: {len(regressions)}")
    for path in regressions[:50]:
        print(f"  - {path}")

    print(f"Improvements: {len(improvements)}")
    for path in improvements[:50]:
        print(f"  + {path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

