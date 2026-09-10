"""Run a verification command and preserve real output/exit status, no environment dump."""
import hashlib
import json
import pathlib
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parents[3]
name, *command = sys.argv[1:]
result = subprocess.run(command, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
folder = pathlib.Path(__file__).resolve().parent
(folder / (name + ".log")).write_text(result.stdout, encoding="utf-8")
inputs = [root / "Cargo.lock", root / "Cargo.toml", root / "crates/tericrypt/Cargo.toml"]
inputs += sorted((root / "crates/tericrypt/src").rglob("*.rs"))
inputs += sorted((root / "crates/tericrypt/tests").rglob("*.rs"))
source_hashes = {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in inputs}
(folder / (name + ".json")).write_text(json.dumps({"command": command, "exit_code": result.returncode, "source_sha256": source_hashes}, indent=2) + "\n", encoding="utf-8")
print(result.stdout)
print("EXIT_CODE=" + str(result.returncode))
sys.exit(result.returncode)
