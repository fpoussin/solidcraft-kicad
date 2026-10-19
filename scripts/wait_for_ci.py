#!/usr/bin/env python3
"""Wait for successful main-branch CI and select its release artifacts by commit."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
from urllib.parse import urlencode

from release import TARGETS


def api(path):
    return json.loads(subprocess.check_output(["gh", "api", path], text=True, timeout=60))


def wait(repository, branch, commit, timeout=7200, interval=20):
    query = urlencode({"branch": branch, "event": "push", "head_sha": commit, "per_page": 100})
    endpoint = f"repos/{repository}/actions/workflows/ci.yml/runs?{query}"
    deadline = time.monotonic() + timeout
    previous = None
    while True:
        runs = [run for run in api(endpoint)["workflow_runs"]
                if run["head_sha"] == commit and run["head_branch"] == branch
                and run["event"] == "push"]
        # Never substitute another commit, a PR build, or an older successful run
        # while a newer run for this commit is queued, running, or failed.
        run = max(runs, key=lambda item: item["id"]) if runs else None
        status = f"CI {run['id']}: {run['status']}" if run else f"Waiting for CI on {branch} at {commit}"
        if status != previous:
            print(status, flush=True)
            previous = status
        if run and run["status"] == "completed":
            if run["conclusion"] != "success":
                raise ValueError(f"CI {run['id']} finished with {run['conclusion']}; release blocked")
            artifacts = api(f"repos/{repository}/actions/runs/{run['id']}/artifacts?per_page=100")["artifacts"]
            selected = []
            for target in TARGETS:
                name = f"binaries-{target}"
                matches = [artifact for artifact in artifacts
                           if artifact["name"] == name and not artifact["expired"]]
                if not matches:
                    raise ValueError(f"CI {run['id']} has no unexpired {name}; rerun main CI for {commit}")
                selected.append(max(matches, key=lambda item: item["id"])["id"])
            return {"run-id": run["id"], "artifact-ids": ",".join(map(str, selected)), "commit": commit}
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(f"Timed out waiting for main CI on {branch} at {commit}")
        time.sleep(min(interval, remaining))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY"))
    parser.add_argument("--branch", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--timeout", type=int, default=7200, help="maximum wait in seconds (default: 2 hours)")
    args = parser.parse_args()
    if not args.repository:
        parser.error("set GITHUB_REPOSITORY or pass --repository")
    result = wait(args.repository, args.branch, args.commit, args.timeout)
    if output := os.environ.get("GITHUB_OUTPUT"):
        with Path(output).open("a", encoding="utf-8") as stream:
            stream.writelines(f"{key}={value}\n" for key, value in result.items())
    print(json.dumps(result))


if __name__ == "__main__":
    main()
