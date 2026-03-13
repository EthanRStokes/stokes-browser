#!/usr/bin/env python3
import argparse
import os
import subprocess
import sys


def run(cmd, cwd=None):
    print("+", " ".join(cmd))
    subprocess.check_call(cmd, cwd=cwd)


def main() -> int:
    parser = argparse.ArgumentParser(description="Bootstrap or update local WPT checkout")
    parser.add_argument("--repo", default="https://github.com/web-platform-tests/wpt.git")
    parser.add_argument("--dest", default="third_party/wpt")
    args = parser.parse_args()

    dest = os.path.abspath(args.dest)
    parent = os.path.dirname(dest)
    os.makedirs(parent, exist_ok=True)

    if not os.path.isdir(dest):
        run(["git", "clone", "--depth", "1", args.repo, dest])
    else:
        run(["git", "fetch", "--depth", "1", "origin"], cwd=dest)
        run(["git", "reset", "--hard", "origin/master"], cwd=dest)

    run([sys.executable, "wpt", "manifest", "--rebuild"], cwd=dest)
    print(f"WPT is ready at {dest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

