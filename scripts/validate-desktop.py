#!/usr/bin/env python3
"""Validate insertion or native-file opening in a disposable SolidCraft desktop."""
import argparse
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", required=True, type=Path)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--board", type=Path)
    source.add_argument("--native", type=Path)
    parser.add_argument("--max-open-seconds", default=3.0, type=float)
    parser.add_argument("--cli", default=Path("target/debug/solidcraft-kicad"), type=Path)
    parser.add_argument("--artifacts", default=Path("artifacts/desktop"), type=Path)
    parser.add_argument("--expected-solids", type=int, default=35)
    args = parser.parse_args()
    root = args.artifacts.absolute()
    root.mkdir(parents=True, exist_ok=True)
    control = root / "control"
    control.mkdir(exist_ok=True)
    app_env = os.environ.copy()
    for variable, folder in [
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CACHE_HOME", "cache"),
    ]:
        path = root / folder
        path.mkdir(exist_ok=True)
        app_env[variable] = str(path)
    app_env["SOLIDCRAFT_CONTROL_DIR"] = str(control)
    app_env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    marker = Path(app_env["XDG_DATA_HOME"]) / "solidcraft" / "no-desktop-integration"
    marker.parent.mkdir(parents=True, exist_ok=True)
    marker.touch()
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    with (root / "app.log").open("w") as log:
        process = subprocess.Popen(
            ["xvfb-run", "-a", "-s", "-screen 0 1600x1000x24",
             str(args.app.absolute()), "--control", str(port)],
            env=app_env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
        )
        try:
            token_file = control / f"{port}.token"
            for _ in range(240):
                if process.poll() is not None:
                    raise RuntimeError("isolated app exited; see app.log")
                if token_file.exists():
                    break
                time.sleep(0.2)
            else:
                raise TimeoutError("isolated app did not start")
            token = token_file.read_text().strip()

            def call(method, params=None):
                with socket.create_connection(("127.0.0.1", port), timeout=15) as connection:
                    connection.settimeout(120)
                    request = {"id": 1, "method": method, "params": params or {}, "token": token}
                    connection.sendall((json.dumps(request) + "\n").encode())
                    with connection.makefile("r") as stream:
                        response = json.loads(stream.readline())
                    if not response.get("ok"):
                        raise RuntimeError(response)
                    return response["result"]

            def execute(command, params=None):
                return call("engine.execute", {"command": command, "params": params or {}})

            version = call("ui.help")["version"]
            print(f"Isolated SolidCraft {version} ready", flush=True)
            if args.native:
                started = time.perf_counter()
                execute("doc.open", {"path": str(args.native.absolute())})
                opened = time.perf_counter() - started
                call("ui.render", {"path": str(root / "model.png"), "width": 1400,
                                   "height": 1000, "grid": False, "axes": False})
                rendered = time.perf_counter() - started
                tree = execute("component.list", {"compact": True})
                call("ui.screenshot", {"path": str(root / "window.png")})
                result = {"version": version, "open_seconds": opened,
                          "open_and_render_seconds": rendered, "components": tree}
                (root / "open-timing.json").write_text(json.dumps(result, indent=2))
                assert rendered <= args.max_open_seconds, result
                print(f"Native file opened in {opened:.3f}s; first render ready in {rendered:.3f}s", flush=True)
                return
            execute("file.new", {"name": "Insertion validation"})
            execute("solid.box", {"length": 2, "width": 3, "height": 4})
            before = execute("inspect.measure")
            cli_env = os.environ.copy()
            # Keep the user's KiCad configuration for model discovery, but use our private token.
            cli_env["SOLIDCRAFT_CONTROL_DIR"] = str(control)
            result = subprocess.run(
                [str(args.cli.absolute()), "insert", str(args.board.absolute()), "--port", str(port), "--json"],
                env=cli_env, capture_output=True, text=True, timeout=180,
            )
            (root / "insert-report.json").write_text(result.stdout)
            (root / "insert-stderr.log").write_text(result.stderr)
            if result.returncode:
                raise RuntimeError(f"insert failed:\n{result.stderr}")
            report = json.loads(result.stdout)
            after = execute("inspect.measure")
            tree = execute("component.list")
            assert after["body_count"] == args.expected_solids + before["body_count"]
            original = before["bodies"][0]
            assert next(body for body in after["bodies"] if body["name"] == original["name"]) == original
            assert execute("component.links")["links"] == []
            execute("file.save_as", {"path": str(root / "inserted.solidcraft")})
            call("ui.camera", {"fit": [report["result"]["occurrence"]]})
            call("ui.set", {"visualStyle": 0})
            execute("select.set", {"items": []})
            time.sleep(0.2)
            handles = call("ui.handles")
            (root / "handles.json").write_text(json.dumps(handles, indent=2))
            rows = handles if isinstance(handles, list) else handles.get("handles", [])
            name = report["result"]["component_name"]
            if not any(handle.get("name", "").startswith("row:PCB:1") for handle in rows):
                for handle in rows:
                    if handle.get("name", "").startswith(f"fold:{name}"):
                        call("ui.click", {"x": handle["x"], "y": handle["y"]})
                        break
            # Show the component tree compactly, with the per-part bodies collapsed.
            for component in tree["components"]:
                if component.get("parent") != report["result"]["component"]:
                    continue
                current = call("ui.handles")
                shown = {handle["name"] for handle in current}
                if not any(f"row:{body}" in shown for body in component["bodies"]):
                    continue
                label = component["occurrences"][0]["name"]
                for handle in current:
                    if handle["name"] == f"fold:{label}":
                        call("ui.click", {"x": handle["x"], "y": handle["y"]})
                        time.sleep(0.1)
                        break
            time.sleep(0.2)
            call("ui.render", {"path": str(root / "model.png"), "width": 1400,
                               "height": 1000, "grid": False, "axes": False})
            call("ui.screenshot", {"path": str(root / "window.png")})
            (root / "validation.json").write_text(json.dumps(
                {"version": version, "before": before, "after": after,
                 "components": tree, "insert": report}, indent=2,
            ))
            print(f"Verified {args.expected_solids} inserted solids; existing solid unchanged; no external links", flush=True)
            print(f"Artifacts: {root}", flush=True)
        finally:
            if "call" in locals():
                try:
                    call("app.quit")
                    process.wait(timeout=5)
                except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired):
                    pass
            try:
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=10)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass


if __name__ == "__main__":
    main()
