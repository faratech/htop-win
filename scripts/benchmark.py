#!/usr/bin/env python3
"""
Benchmark script for htop-win.
Runs the benchmark mode and extracts clean timing statistics.

Usage:
    python scripts/benchmark.py [iterations]
    python scripts/benchmark.py 20          # Run 20 iterations
    python scripts/benchmark.py --compare   # Compare with/without efficiency mode
"""

import subprocess
import sys
import re
import os
from pathlib import Path

def strip_ansi(text: str) -> str:
    """Remove ANSI escape codes from text."""
    ansi_escape = re.compile(r'\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])')
    return ansi_escape.sub('', text)

def run_benchmark(iterations: int = 20, extra_args: list = None) -> dict:
    """Run htop-win in benchmark mode and return parsed results."""
    project_root = Path(__file__).parent.parent
    exe_path = project_root / "target" / "release" / "htop-win.exe"

    if not exe_path.exists():
        print(f"Error: {exe_path} not found. Run 'cargo build --release' first.")
        sys.exit(1)

    cmd = [str(exe_path), "--benchmark", str(iterations)]
    if extra_args:
        cmd.extend(extra_args)

    print(f"Running: {' '.join(cmd)}")
    result = subprocess.run(cmd, capture_output=True, text=True, encoding='utf-8', errors='replace')

    # Combine stdout and stderr, strip ANSI codes
    output = strip_ansi(result.stdout + result.stderr)

    # Parse the benchmark results
    stats = {}

    # Extract iterations and process count
    match = re.search(r'Iterations:\s*(\d+)\s*Processes:\s*(\d+)', output)
    if match:
        stats['iterations'] = int(match.group(1))
        stats['processes'] = int(match.group(2))

    # Extract REFRESH stats
    refresh_match = re.search(
        r'REFRESH.*?Total:\s*([\d.]+)(\w+).*?Avg:\s*([\d.]+)(\w+).*?Min:\s*([\d.]+)(\w+).*?Max:\s*([\d.]+)(\w+)',
        output, re.DOTALL
    )
    if refresh_match:
        stats['refresh'] = {
            'total': f"{refresh_match.group(1)}{refresh_match.group(2)}",
            'avg': f"{refresh_match.group(3)}{refresh_match.group(4)}",
            'min': f"{refresh_match.group(5)}{refresh_match.group(6)}",
            'max': f"{refresh_match.group(7)}{refresh_match.group(8)}",
        }

    # Extract DRAW stats
    draw_match = re.search(
        r'DRAW.*?Total:\s*([\d.]+)(\w+).*?Avg:\s*([\d.]+)(\w+).*?Min:\s*([\d.]+)(\w+).*?Max:\s*([\d.]+)(\w+)',
        output, re.DOTALL
    )
    if draw_match:
        stats['draw'] = {
            'total': f"{draw_match.group(1)}{draw_match.group(2)}",
            'avg': f"{draw_match.group(3)}{draw_match.group(4)}",
            'min': f"{draw_match.group(5)}{draw_match.group(6)}",
            'max': f"{draw_match.group(7)}{draw_match.group(8)}",
        }

    # Extract OVERALL stats
    wall_match = re.search(r'Wall time:\s*([\d.]+)(\w+)', output)
    cpu_time_match = re.search(r'CPU time:\s*([\d.]+)(\w+)', output)
    cpu_usage_match = re.search(r'CPU usage:\s*([\d.]+)%', output)

    if wall_match:
        stats['wall_time'] = f"{wall_match.group(1)}{wall_match.group(2)}"
    if cpu_time_match:
        stats['cpu_time'] = f"{cpu_time_match.group(1)}{cpu_time_match.group(2)}"
    if cpu_usage_match:
        stats['cpu_usage'] = float(cpu_usage_match.group(1))

    return stats

def print_stats(stats: dict, label: str = ""):
    """Pretty print benchmark statistics."""
    if label:
        print(f"\n{'='*60}")
        print(f"  {label}")
        print(f"{'='*60}")

    print(f"\n  Iterations: {stats.get('iterations', '?')}  |  Processes: {stats.get('processes', '?')}")

    if 'refresh' in stats:
        r = stats['refresh']
        print(f"\n  REFRESH (system data collection)")
        print(f"    Total: {r['total']:>12}   Avg: {r['avg']:>12}")
        print(f"    Min:   {r['min']:>12}   Max: {r['max']:>12}")

    if 'draw' in stats:
        d = stats['draw']
        print(f"\n  DRAW (UI rendering)")
        print(f"    Total: {d['total']:>12}   Avg: {d['avg']:>12}")
        print(f"    Min:   {d['min']:>12}   Max: {d['max']:>12}")

    print(f"\n  OVERALL")
    print(f"    Wall time:  {stats.get('wall_time', '?'):>12}")
    print(f"    CPU time:   {stats.get('cpu_time', '?'):>12}")
    print(f"    CPU usage:  {stats.get('cpu_usage', '?'):>11.1f}%")
    print()

def compare_mode(iterations: int = 15):
    """Compare benchmark results with and without efficiency mode."""
    print("\n" + "="*60)
    print("  BENCHMARK COMPARISON")
    print("="*60)

    # Run with efficiency mode (default)
    print("\n>>> Running with Efficiency Mode (default)...")
    with_eff = run_benchmark(iterations)

    # Run without efficiency mode
    print("\n>>> Running without Efficiency Mode (--inefficient)...")
    without_eff = run_benchmark(iterations, ['--inefficient'])

    # Print comparison
    print_stats(with_eff, "WITH Efficiency Mode")
    print_stats(without_eff, "WITHOUT Efficiency Mode (--inefficient)")

    # Summary comparison
    if 'cpu_usage' in with_eff and 'cpu_usage' in without_eff:
        diff = without_eff['cpu_usage'] - with_eff['cpu_usage']
        print(f"\n  CPU Usage Difference: {diff:+.1f}% ({'higher' if diff > 0 else 'lower'} without efficiency)")

def main():
    iterations = 20
    compare = False

    for arg in sys.argv[1:]:
        if arg == '--compare':
            compare = True
        elif arg.isdigit():
            iterations = int(arg)
        elif arg in ['-h', '--help']:
            print(__doc__)
            sys.exit(0)

    if compare:
        compare_mode(iterations)
    else:
        stats = run_benchmark(iterations)
        print_stats(stats, f"BENCHMARK RESULTS ({iterations} iterations)")

if __name__ == '__main__':
    main()
