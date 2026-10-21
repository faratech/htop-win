#!/usr/bin/env python3
"""Compare CPU utilization between old and new htop-win versions."""

import subprocess
import argparse
import math
import shutil
from pathlib import Path


def find_powershell():
    for name in ("powershell", "powershell.exe", "pwsh"):
        shell = shutil.which(name)
        if shell:
            return shell
    raise RuntimeError("PowerShell is required to measure Windows process CPU time")


def powershell_literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def measure_cpu_time(exe_path, iterations=20, delay=100, runs=3):
    """Run exe multiple times and measure CPU time."""
    shell = find_powershell()
    arguments = [
        "--max-iterations", str(iterations), "--delay", str(delay),
        "--no-mouse", "--sort", "pid", "--no-auto-update",
    ]
    argument_list = powershell_literal(subprocess.list2cmdline(arguments))
    results = []
    for i in range(runs):
        cmd = f'''
$ErrorActionPreference = 'Stop'
$start = New-Object System.Diagnostics.ProcessStartInfo
$start.FileName = {powershell_literal(exe_path)}
$start.Arguments = {argument_list}
$start.UseShellExecute = $false
# htop needs console input; inherit the wrapper's console while suppressing
# any new console window. Direct process creation bypasses shell associations.
$start.CreateNoWindow = $false
$start.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$proc = New-Object System.Diagnostics.Process
$proc.StartInfo = $start
if (-not $proc.Start()) {{
    throw 'Unable to start benchmark executable'
}}
$stdout = $proc.StandardOutput.ReadToEndAsync()
$stderr = $proc.StandardError.ReadToEndAsync()
# Keep a process handle before waiting so short-lived children retain exit data.
$null = $proc.Handle
$proc.WaitForExit()
$childOutput = $stdout.GetAwaiter().GetResult()
$childError = $stderr.GetAwaiter().GetResult()
if ($proc.ExitCode -ne 0) {{
    throw "Benchmark executable exited with code $($proc.ExitCode)`n$childOutput`n$childError"
}}
$proc.TotalProcessorTime.TotalMilliseconds.ToString('R', [Globalization.CultureInfo]::InvariantCulture)
'''
        result = subprocess.run(
            [shell, '-NoProfile', '-NonInteractive', '-Command', cmd],
            capture_output=True, text=True
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"Run {i + 1} failed for {exe_path} with exit code {result.returncode}\n"
                f"stdout:\n{result.stdout}\n"
                f"stderr:\n{result.stderr}"
            )
        try:
            cpu_ms = float(result.stdout.strip())
        except ValueError as exc:
            raise RuntimeError(
                f"Run {i + 1} did not return numeric CPU time for {exe_path}: "
                f"{result.stdout!r}"
            ) from exc
        if not math.isfinite(cpu_ms) or cpu_ms < 0:
            raise RuntimeError(
                f"Run {i + 1} did not return finite nonnegative CPU time for {exe_path}: "
                f"{result.stdout!r}"
            )
        results.append(cpu_ms)
        print(f"  Run {i+1}: {cpu_ms:.0f}ms")
    if not results:
        raise RuntimeError(f"No successful benchmark runs for {exe_path}")
    return results

def main():
    parser = argparse.ArgumentParser(description="Compare CPU utilization between two htop-win executables.")
    parser.add_argument("old_exe", help="Path to the baseline executable")
    parser.add_argument("new_exe", help="Path to the new executable")
    parser.add_argument("--iterations", type=int, default=20)
    parser.add_argument("--delay", type=int, default=100)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--old-label", default="Baseline")
    parser.add_argument("--new-label", default="Candidate")
    args = parser.parse_args()
    old_exe = args.old_exe
    new_exe = args.new_exe
    for path in (old_exe, new_exe):
        if not Path(path).exists():
            raise SystemExit(f"Executable not found: {path}")

    print("=" * 60)
    print(f"  CPU UTILIZATION COMPARISON ({args.iterations} iterations, {args.delay}ms delay)")
    print("=" * 60)

    print(f"\n{args.old_label.upper()}:")
    old_results = measure_cpu_time(old_exe, args.iterations, args.delay, args.runs)
    old_avg = sum(old_results) / len(old_results)

    print(f"\n{args.new_label.upper()}:")
    new_results = measure_cpu_time(new_exe, args.iterations, args.delay, args.runs)
    new_avg = sum(new_results) / len(new_results)

    print("\n" + "=" * 60)
    print("  SUMMARY")
    print("=" * 60)
    print(f"\n  {args.old_label:<18} {old_avg:>8.0f}ms avg CPU time")
    print(f"  {args.new_label:<18} {new_avg:>8.0f}ms avg CPU time")

    if old_avg > 0:
        reduction = ((old_avg - new_avg) / old_avg) * 100
        print(f"\n  Improvement:       {reduction:>8.1f}% less CPU time")

    print()

if __name__ == '__main__':
    main()
