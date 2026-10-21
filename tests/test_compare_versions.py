import importlib.util
import math
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch


MODULE_PATH = Path(__file__).resolve().parents[1] / "scripts" / "compare_versions.py"
SPEC = importlib.util.spec_from_file_location("compare_versions", MODULE_PATH)
compare_versions = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(compare_versions)


class SampleValidationTests(unittest.TestCase):
    def test_invalid_samples_are_rejected(self):
        for sample in ("nan", "inf", "-inf", "-1", "not a number"):
            with self.subTest(sample=sample):
                completed = subprocess.CompletedProcess([], 0, sample, "")
                with patch.object(compare_versions, "find_powershell", return_value="powershell"):
                    with patch.object(compare_versions.subprocess, "run", return_value=completed):
                        with self.assertRaises(RuntimeError):
                            compare_versions.measure_cpu_time("fixture.exe", runs=1)

    def test_wrapper_failure_does_not_accept_numeric_stdout(self):
        completed = subprocess.CompletedProcess([], 1, "12.5", "wrapper failed")
        with patch.object(compare_versions, "find_powershell", return_value="powershell"):
            with patch.object(compare_versions.subprocess, "run", return_value=completed):
                with self.assertRaisesRegex(RuntimeError, "wrapper failed"):
                    compare_versions.measure_cpu_time("fixture.exe", runs=1)


class PowerShellProcessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        try:
            cls.shell = compare_versions.find_powershell()
        except RuntimeError as error:
            raise unittest.SkipTest(str(error)) from error
        result = subprocess.run(
            [cls.shell, "-NoProfile", "-NonInteractive", "-Command",
             "[Environment]::OSVersion.Platform.ToString()"],
            capture_output=True, text=True, check=True, timeout=30,
        )
        windows_shell = result.stdout.strip() == "Win32NT"
        wslpath = shutil.which("wslpath")
        if windows_shell and os.name != "nt" and wslpath is None:
            raise unittest.SkipTest("Windows PowerShell on this host needs wslpath for fixtures")

        fixture_parent = None
        if windows_shell:
            windows_temp = subprocess.run(
                [cls.shell, "-NoProfile", "-NonInteractive", "-Command",
                 "[System.IO.Path]::GetTempPath()"],
                capture_output=True, text=True, check=True, timeout=30,
            ).stdout.strip()
            if os.name == "nt":
                fixture_parent = windows_temp
            else:
                fixture_parent = subprocess.run(
                    [wslpath, "-u", windows_temp], capture_output=True,
                    text=True, check=True, timeout=10,
                ).stdout.strip()
        cls.fixture_directory = tempfile.TemporaryDirectory(
            prefix="htop-compare-native-test-", dir=fixture_parent,
        )
        cls.addClassCleanup(cls.fixture_directory.cleanup)
        cls.root = Path(cls.fixture_directory.name)
        cls.fixture = cls.root / "fixture.exe"
        compiler = shutil.which("rustc")
        if compiler is None:
            raise unittest.SkipTest("Native process fixtures require rustc")
        command = [compiler, "-", "--crate-name", "compare_fixture", "--edition", "2024",
                   "-C", "target-feature=+crt-static", "-o", str(cls.fixture)]
        if windows_shell and os.name != "nt":
            command += ["--target", "x86_64-pc-windows-gnu"]
        # A GUI-subsystem fixture never creates a console or displays UI. Its
        # only effects are an argument-recording file and a chosen exit code.
        source = r'''
#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    let executable = std::env::current_exe().unwrap();
    if executable.file_name().unwrap().to_string_lossy().starts_with("failure") {
        std::process::exit(1);
    }
    let arguments = std::env::args().skip(1).collect::<Vec<_>>().join("\n");
    std::fs::write(executable.with_extension("args"), arguments).unwrap();
}
'''
        subprocess.run(command, input=source, capture_output=True, text=True,
                       check=True, timeout=60)
        cls.windows_shell = windows_shell
        cls.wslpath = wslpath

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="case-", dir=self.root)
        self.addCleanup(self.directory.cleanup)
        self.case_root = Path(self.directory.name)

    def shell_path(self, path):
        if self.windows_shell and os.name != "nt" and self.wslpath:
            return subprocess.run(
                [self.wslpath, "-w", str(path)], capture_output=True,
                text=True, check=True, timeout=10,
            ).stdout.strip()
        return str(path)

    def executable(self, name):
        path = self.case_root / (name + ".exe")
        shutil.copyfile(self.fixture, path)
        path.chmod(0o755)
        return self.shell_path(path)

    def test_nonzero_child_exit_is_rejected(self):
        executable = self.executable("failure")
        with self.assertRaisesRegex(RuntimeError, "exited with code 1"):
            compare_versions.measure_cpu_time(executable, iterations=1, runs=1)

    def test_missing_executable_is_a_wrapper_failure(self):
        executable = self.shell_path(self.case_root / "missing executable.exe")
        with self.assertRaisesRegex(RuntimeError, "failed.*exit code"):
            compare_versions.measure_cpu_time(executable, iterations=1, runs=1)

    def test_quoted_path_success_and_consistent_workload_arguments(self):
        arguments_file = self.case_root / "sample's success.args"
        executable = self.executable("sample's success")
        samples = compare_versions.measure_cpu_time(executable, iterations=2, runs=1)
        self.assertEqual(len(samples), 1)
        self.assertTrue(math.isfinite(samples[0]))
        self.assertGreaterEqual(samples[0], 0)
        self.assertEqual(
            arguments_file.read_text().split(),
            ["--max-iterations", "2", "--delay", "100", "--no-mouse",
             "--sort", "pid", "--no-auto-update"],
        )


if __name__ == "__main__":
    unittest.main()
