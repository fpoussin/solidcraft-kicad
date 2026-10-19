"""Exercise tag/main races and release gates without sleeping or publishing."""
import contextlib
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from release import TARGETS
import wait_for_ci as ci


SHA = "a" * 40


def run(identifier=10, status="completed", conclusion="success", **overrides):
    return {"id": identifier, "status": status, "conclusion": conclusion,
            "head_sha": SHA, "head_branch": "master", "event": "push", **overrides}


def artifacts():
    return [{"id": index, "name": f"binaries-{target}", "expired": False}
            for index, target in enumerate(TARGETS, 100)]


class WaitForCiTests(unittest.TestCase):
    def setUp(self):
        self.clock = 0
        self.addCleanup(patch.stopall)
        patch.object(ci.time, "monotonic", side_effect=lambda: self.clock).start()
        self.sleep = patch.object(ci.time, "sleep", side_effect=self.advance).start()
        self.api = patch.object(ci, "api").start()

    def advance(self, seconds):
        self.clock += seconds

    def wait(self, **kwargs):
        with contextlib.redirect_stdout(io.StringIO()):
            return ci.wait("owner/repo", "master", SHA, interval=1, **{"timeout": 5, **kwargs})

    def test_tag_can_arrive_before_main_ci_is_created(self):
        self.api.side_effect = [
            {"workflow_runs": []},
            {"workflow_runs": [run(status="queued", conclusion=None)]},
            {"workflow_runs": [run(status="in_progress", conclusion=None)]},
            {"workflow_runs": [run()]},
            {"artifacts": artifacts()},
        ]
        self.assertEqual(self.wait(), {"run-id": 10, "artifact-ids": "100,101,102", "commit": SHA})
        self.assertEqual(self.sleep.call_count, 3)
        endpoint = self.api.call_args_list[0].args[0]
        self.assertIn("/workflows/ci.yml/runs?", endpoint)
        for query in ("branch=master", "event=push", f"head_sha={SHA}"):
            self.assertIn(query, endpoint)

    def test_already_successful_main_ci_is_reused_immediately(self):
        self.api.side_effect = [{"workflow_runs": [run()]}, {"artifacts": artifacts()}]
        self.assertEqual(self.wait()["run-id"], 10)
        self.sleep.assert_not_called()

    def test_other_commits_pull_requests_and_branches_never_match(self):
        self.api.return_value = {"workflow_runs": [run(head_sha="b" * 40),
                                                  run(event="pull_request"), run(head_branch="feature")]}
        with self.assertRaises(TimeoutError):
            self.wait(timeout=2)
        self.assertEqual(self.clock, 2)

    def test_newer_queued_run_takes_precedence_over_older_success(self):
        self.api.side_effect = [
            {"workflow_runs": [run(), run(20, status="queued", conclusion=None)]},
            {"workflow_runs": [run(), run(20)]},
            {"artifacts": artifacts()},
        ]
        self.assertEqual(self.wait()["run-id"], 20)
        self.assertIn("/runs/20/artifacts", self.api.call_args.args[0])

    def test_unsuccessful_ci_blocks_publication_even_with_older_success(self):
        for conclusion in ("failure", "cancelled", "timed_out", "neutral", "skipped", "action_required"):
            with self.subTest(conclusion=conclusion):
                self.api.return_value = {"workflow_runs": [run(), run(20, conclusion=conclusion)]}
                with self.assertRaisesRegex(ValueError, "release blocked"):
                    self.wait()
        self.sleep.assert_not_called()

    def test_each_platform_archive_must_exist_and_be_unexpired(self):
        for index in range(len(TARGETS)):
            for missing in (True, False):
                with self.subTest(target=TARGETS[index], missing=missing):
                    files = artifacts()
                    if missing:
                        files.pop(index)
                    else:
                        files[index]["expired"] = True
                    self.api.side_effect = [{"workflow_runs": [run()]}, {"artifacts": files}]
                    with self.assertRaisesRegex(ValueError, "rerun main CI"):
                        self.wait()

    def test_rerun_selects_newest_live_artifact_and_ignores_timings(self):
        files = artifacts() + [{"id": 200, "name": f"binaries-{TARGETS[0]}", "expired": False},
                               {"id": 300, "name": "timings-linux", "expired": False}]
        self.api.side_effect = [{"workflow_runs": [run()]}, {"artifacts": files}]
        self.assertEqual(self.wait()["artifact-ids"], "200,101,102")

    def test_pending_ci_times_out(self):
        self.api.return_value = {"workflow_runs": [run(status="in_progress", conclusion=None)]}
        with self.assertRaises(TimeoutError):
            self.wait(timeout=2)
        self.assertEqual(self.clock, 2)

    def test_cli_writes_run_and_artifact_ids_for_downstream_download(self):
        self.api.side_effect = [{"workflow_runs": [run()]}, {"artifacts": artifacts()}]
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "outputs"
            with patch.dict(os.environ, {"GITHUB_REPOSITORY": "owner/repo", "GITHUB_OUTPUT": str(output)}), \
                 patch("sys.argv", ["wait_for_ci.py", "--branch", "master", "--commit", SHA]), \
                 contextlib.redirect_stdout(io.StringIO()):
                ci.main()
            self.assertEqual(output.read_text(), f"run-id=10\nartifact-ids=100,101,102\ncommit={SHA}\n")


if __name__ == "__main__":
    unittest.main()
