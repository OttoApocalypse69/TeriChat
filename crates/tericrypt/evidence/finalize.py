"""Bind handoff paths and final command results to the actual worktree contents."""
import hashlib
import json
import pathlib
import re
import subprocess
import tomllib

root = pathlib.Path(__file__).resolve().parents[3]
folder = pathlib.Path(__file__).resolve().parent

def git(*args):
    return subprocess.check_output(["git", *args], cwd=root, text=True, encoding="utf-8").strip()

head = git("rev-parse", "HEAD")
branch = git("branch", "--show-current")
assert head == "5cf0a590c0298be198eee7095053f0f736d150fb"
assert branch == "feat/6-mls-foundation"
paths = sorted(set(git("diff", "--name-only").splitlines() + git("ls-files", "--others", "--exclude-standard").splitlines() + ["crates/tericrypt/evidence/manifest.json"]))
assert all(p == "Cargo.lock" or p.startswith("crates/tericrypt/") for p in paths)
assert git("diff", "--", "Cargo.toml") == ""
results = {}
for path in sorted(folder.glob("final-*.json")):
    result = json.loads(path.read_text(encoding="utf-8"))
    assert result["exit_code"] == 0, path
    for source, digest in result["source_sha256"].items():
        assert hashlib.sha256((root/source).read_bytes()).hexdigest() == digest, source
    results[path.stem] = result
counts = [int(n) for n in re.findall(r"test result: ok\. (\d+) passed", (folder / "final-tests.log").read_text(encoding="utf-8"))]
base = tomllib.loads(git("show", "HEAD:Cargo.lock"))["package"]
assert any(p["name"] == "rsa" and p["version"] == "0.9.10" for p in base)
assert not any(p["name"] == "proc-macro-error2" for p in base)
manifest = {
    "task": "issue-6-first-slice", "state": "parent-verification-and-independent-review-pending",
    "branch": branch, "base": head, "head": head, "worktree": root.as_posix(),
    "changed_paths": paths, "root_manifest_unchanged": True,
    "passed_feature_tests": sum(counts), "feature_test_suite_counts": counts,
    "final_commands": results,
    "audit_exit_code": json.loads((folder / "audit.json").read_text())["exit_code"],
    "audit_findings": ["baseline rsa 0.9.10 RUSTSEC-2023-0071", "added lockfile proc-macro-error2 2.0.1 RUSTSEC-2026-0173 unmaintained warning"],
    "limitations": ["synthetic client-local only", "non-durable storage", "no credential trust/bootstrap", "no production protocol migration", "audit not green", "no cross-platform validation", "independent specialist review pending"],
    "file_sha256": {p: hashlib.sha256((root/p).read_bytes()).hexdigest() for p in paths if not p.endswith("/manifest.json")},
}
(folder / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
print(json.dumps({"manifest": (folder / "manifest.json").as_posix(), "changed_path_count": len(paths), "passed_feature_tests": sum(counts), "final_command_count": len(results), "audit_exit_code": manifest["audit_exit_code"], "scope_and_hashes": "PASS"}, indent=2))
