"""Record resolved dependencies/licenses and verify owned paths without reading environment."""
import hashlib
import json
import pathlib
import subprocess
import tomllib

root = pathlib.Path(__file__).resolve().parents[3]
evidence = pathlib.Path(__file__).resolve().parent

def run(*args):
    return subprocess.check_output(args, cwd=root, text=True, encoding="utf-8")

metadata = json.loads(run("cargo", "metadata", "--locked", "--format-version", "1", "--features", "tericrypt/mls-foundation", "--filter-platform", "x86_64-pc-windows-msvc"))
packages = {p["id"]: p for p in metadata["packages"]}
nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
start = next(i for i, p in packages.items() if p["name"] == "tericrypt")
seen, todo = set(), [start]
while todo:
    ident = todo.pop()
    if ident in seen:
        continue
    seen.add(ident)
    todo.extend(nodes[ident]["dependencies"])
report = sorted([{
    "name": packages[i]["name"], "version": packages[i]["version"],
    "license": packages[i]["license"], "rust_version": packages[i]["rust_version"],
    "repository": packages[i]["repository"], "features": nodes[i]["features"]
} for i in seen], key=lambda p: (p["name"], p["version"]))
(evidence / "dependencies.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
base = tomllib.loads(run("git", "show", "HEAD:Cargo.lock"))["package"]
current = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))["package"]
old = {(p["name"], p["version"]) for p in base}
new = {(p["name"], p["version"]) for p in current}
lock_report = {"added": sorted(new-old), "removed": sorted(old-new)}
(evidence / "lock-changes.json").write_text(json.dumps(lock_report, indent=2) + "\n", encoding="utf-8")
print(json.dumps({"focus": [p for p in report if p["name"].startswith(("openmls", "hpke", "tls_codec"))], "removed_lock_packages": lock_report["removed"], "resolved_dependency_count": len(report)}, indent=2))
paths = sorted(set(run("git", "diff", "--name-only").splitlines() + run("git", "ls-files", "--others", "--exclude-standard").splitlines()))
assert all(p == "Cargo.lock" or p.startswith("crates/tericrypt/") for p in paths), paths
print("Owned-path check: PASS")
