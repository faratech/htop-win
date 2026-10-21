"""Maintenance tools must publish the current artifact and keep locked builds valid."""

import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]


def load_tool(filename):
    spec = importlib.util.spec_from_file_location(filename, ROOT / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class CrossBuildTests(unittest.TestCase):
    def test_build_and_copy_use_the_same_directory_even_with_stale_default_output(self):
        for override in (None, "relative-cache", "absolute"):
            with self.subTest(override=override), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                tool = load_tool("build-cross.py")
                tool.PROJECT_DIR = root
                tool.TARGET_DIR = root / "target"
                tool.OUTPUT_DIR = root / "published"
                environment = {"PATH": os.environ.get("PATH", os.defpath)}
                if override:
                    environment["CARGO_TARGET_DIR"] = str(root / "other-cache") if override == "absolute" else override
                expected = root / ({None: "target", "relative-cache": "relative-cache", "absolute": "other-cache"}[override])
                stale = tool.TARGET_DIR / "x86_64-pc-windows-gnu/release/htop-win.exe"
                stale.parent.mkdir(parents=True)
                stale.write_bytes(b"stale executable")
                commands = []

                def cargo(command, **kwargs):
                    commands.append(command)
                    directory = Path(command[command.index("--target-dir") + 1])
                    self.assertEqual(directory, expected)
                    self.assertEqual(kwargs["cwd"], root)
                    if command[1] == "build":
                        artifact = directory / "x86_64-pc-windows-gnu/release/htop-win.exe"
                        artifact.parent.mkdir(parents=True, exist_ok=True)
                        artifact.write_bytes(b"fresh executable")
                    return True

                with mock.patch.dict(os.environ, environment, clear=True), mock.patch.object(tool, "run_command", side_effect=cargo):
                    self.assertTrue(tool.build_target("x64"))
                    published = tool.copy_to_output("x64")
                    self.assertEqual(published.read_bytes(), b"fresh executable")
                    self.assertTrue(tool.check_target("x64"))
                self.assertEqual(len(commands), 2)


class VersionBumpTests(unittest.TestCase):
    def fixture(self, root):
        (root / "src").mkdir()
        (root / "src/lib.rs").write_text("")
        (root / "media").mkdir()
        (root / "media/htop.rc").write_text('FILEVERSION 0,1,0,0\nVALUE "FileVersion", "0.1.0"\n')
        (root / "Cargo.toml").write_text('[package]\nname = "bump-fixture"\nversion = "0.1.0"\nedition = "2024"\n')
        shutil.copyfile(ROOT / "bump-version.py", root / "bump-version.py")
        subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=root, check=True, capture_output=True)

    def run_bump(self, root, *arguments):
        return subprocess.run([sys.executable, str(root / "bump-version.py"), *arguments], cwd=root, capture_output=True, text=True)

    def version_files(self, root):
        return {name: (root / name).read_bytes() for name in ("Cargo.toml", "Cargo.lock", "media/htop.rc")}

    def test_bump_updates_lockfile_and_dry_run_preserves_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            before = self.version_files(root)
            dry = self.run_bump(root, "--dry-run")
            self.assertEqual(dry.returncode, 0, dry.stderr)
            self.assertEqual(before, self.version_files(root))
            result = self.run_bump(root)
            self.assertEqual(result.returncode, 0, result.stderr)
            metadata = subprocess.run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"], cwd=root, capture_output=True, text=True)
            self.assertEqual(metadata.returncode, 0, metadata.stderr)
            import json
            self.assertEqual(json.loads(metadata.stdout)["packages"][0]["version"], "0.1.1")

    def test_resolution_failure_restores_manifest_resource_and_lockfile(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            with (root / "Cargo.toml").open("a") as manifest:
                manifest.write('[dependencies]\nmissing = { path = "does-not-exist" }\n')
            before = self.version_files(root)
            result = self.run_bump(root)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("original version files restored", result.stderr)
            self.assertNotIn("Done!", result.stdout)
            self.assertEqual(before, self.version_files(root))


if __name__ == "__main__":
    unittest.main()
