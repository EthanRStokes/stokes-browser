#!/usr/bin/env python3
import argparse
import os
import shutil


def main() -> int:
    parser = argparse.ArgumentParser(description="Copy latest WPT result to baseline")
    parser.add_argument("--source", required=True)
    parser.add_argument("--dest", required=True)
    args = parser.parse_args()

    source = os.path.abspath(args.source)
    dest = os.path.abspath(args.dest)

    if not os.path.isfile(source):
        raise FileNotFoundError(f"Source file does not exist: {source}")

    os.makedirs(os.path.dirname(dest), exist_ok=True)
    shutil.copyfile(source, dest)
    print(f"Baseline updated: {dest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

