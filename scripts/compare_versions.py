#!/usr/bin/env python3
"""Compare CPU utilization between old and new htop-win versions."""

import subprocess
import time
import os

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
        try:
            cpu_ms = float(result.stdout.strip())
            results.append(cpu_ms)
            print(f"  Run {i+1}: {cpu_ms:.0f}ms")
        except:
            print(f"  Run {i+1}: Error - {result.stderr}")
    return results

def main():
    old_exe = r"D:\htop-win\temp_release\htop-win-amd64.exe"
    new_exe = r"D:\htop-win\target\release\htop-win.exe"

    print("=" * 60)
    print("  CPU UTILIZATION COMPARISON (20 iterations, 100ms delay)")
    print("=" * 60)

    print("\nOLD RELEASE (v0.0.3 with sysinfo):")
    old_results = measure_cpu_time(old_exe)
    old_avg = sum(old_results) / len(old_results) if old_results else 0

    print("\nNEW BUILD (native Windows APIs):")
    new_results = measure_cpu_time(new_exe)
    new_avg = sum(new_results) / len(new_results) if new_results else 0

    print("\n" + "=" * 60)
    print("  SUMMARY")
    print("=" * 60)
    print(f"\n  Old (sysinfo):     {old_avg:>8.0f}ms avg CPU time")
    print(f"  New (native):      {new_avg:>8.0f}ms avg CPU time")

    if old_avg > 0:
        reduction = ((old_avg - new_avg) / old_avg) * 100
        print(f"\n  Improvement:       {reduction:>8.1f}% less CPU time")

    print()

if __name__ == '__main__':
    main()
