#!/usr/bin/env python3
"""Compare CPU utilization between old and new htop-win versions."""

import subprocess
import time
import os
import argparse
from pathlib import Path

def measure_cpu_time(exe_path, iterations=20, delay=100, runs=3):
    """Run exe multiple times and measure CPU time."""
    results = []
    for i in range(runs):
        cmd = f'''
$proc = Start-Process -FilePath '{exe_path}' -ArgumentList '--max-iterations','{iterations}','--delay','{delay}','--no-mouse' -PassThru -WindowStyle Hidden
$proc.WaitForExit()
$proc.TotalProcessorTime.TotalMilliseconds
'''
        result = subprocess.run(
            ['powershell', '-Command', cmd],
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
            results.append(cpu_ms)
            print(f"  Run {i+1}: {cpu_ms:.0f}ms")
        except ValueError as exc:
            raise RuntimeError(
                f"Run {i + 1} did not return numeric CPU time for {exe_path}: "
                f"{result.stdout!r}"
            ) from exc
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
