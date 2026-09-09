"""Verify existing release bytes. This does not build or sign application code."""
import datetime
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import zipfile


def run(args):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True,
                            encoding="utf-8", errors="replace")
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)
    return result.stdout


def main():
    tag = os.environ["RELEASE_TAG"]
    repo = os.environ["GITHUB_REPOSITORY"]
    manifest = json.loads(Path(".github/release-verification-manifest.json").read_text())
    if repo != manifest["repository"]:
        raise RuntimeError("Repository does not match reviewed manifest")
    records = manifest["releases"][tag]
    sdk = Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / "Windows Kits/10/bin"
    candidates = list(sdk.glob("*/x64/signtool.exe"))
    signtool = max(candidates, key=lambda p: tuple(int(v) for v in p.parent.parent.name.split(".")))
    assets = Path("release-assets")
    assets.mkdir(exist_ok=False)
    checks = []
    with tempfile.TemporaryDirectory(prefix="release-verification-") as scratch:
        scratch = Path(scratch)

        def inspect(data, name, label, depth=0):
            if depth > 4:
                raise RuntimeError("Unexpected archive nesting")
            suffix = Path(name).suffix.lower()
            if suffix in (".exe", ".dll", ".msix", ".msixbundle"):
                path = scratch / (str(len(checks)) + suffix)
                path.write_bytes(data)
                output = run([signtool, "verify", "/pa", "/all", "/v", path])
                personal = suffix in (".msix", ".msixbundle")
                expected = ["Mike Fara"] if personal else ["Fara Technologies LLC", "Mike Fara"]
                # Read each signing chain's leaf, excluding timestamp chains.
                chains = re.findall(r"Signing Certificate Chain:\s*([\s\S]*?)(?:The signature is timestamped|File is not timestamped|Successfully verified)", output)
                leaves = []
                for chain in chains:
                    names = re.findall(r"Issued to:\s*([^\r\n]+)", chain)
                    if names:
                        leaves.append(names[-1].strip())
                count = re.search(r"Number of signatures successfully Verified:\s*(\d+)", output)
                if leaves != expected or not count or int(count.group(1)) != len(expected):
                    raise RuntimeError(f"Unexpected signatures for {label}: {leaves}\n{output}")
                checks.append({"path": label, "check": "signtool verify /pa /all /v", "publishers": leaves})
            if suffix in (".zip", ".msix", ".msixbundle"):
                found = 0
                with zipfile.ZipFile(io.BytesIO(data)) as archive:
                    for entry in archive.infolist():
                        if Path(entry.filename).suffix.lower() in (".exe", ".dll", ".msix"):
                            # Do not extract archive-controlled paths onto the filesystem.
                            inspect(archive.read(entry), entry.filename, label + "!/" + entry.filename, depth + 1)
                            found += 1
                if not found:
                    raise RuntimeError(f"No executable/package members in {label}")

        for record in records:
            name = record["name"]
            if Path(name).name != name or name in (".", ".."):
                raise RuntimeError("Invalid manifest filename")
            run(["gh", "release", "download", tag, "--repo", repo, "--dir", assets, "--pattern", name])
            data = (assets / name).read_bytes()
            if hashlib.sha256(data).hexdigest() != record["sha256"]:
                raise RuntimeError(f"Release hash differs from reviewed manifest: {name}")
            inspect(data, name, name)
            print(f"Verified {name}: SHA256 {record['sha256']}", flush=True)
    predicate = {
        "schemaVersion": 1,
        "verificationKind": "existing-release-hash-and-authenticode",
        "repository": repo,
        "releaseTag": tag,
        "verifiedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "workflowCommit": os.environ["GITHUB_SHA"],
        "runUrl": f"https://github.com/{repo}/actions/runs/{os.environ['GITHUB_RUN_ID']}",
        "assets": records,
        "signatureChecks": checks,
        "limitations": "These files were previously re-signed outside this workflow. No compilation, source-to-binary correspondence, or original build provenance is attested. Checksum subjects are hash-checked only."
    }
    Path("release-verification-predicate.json").write_text(json.dumps(predicate, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
