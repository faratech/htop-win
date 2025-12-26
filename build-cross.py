#!/usr/bin/env python3
"""
Cross-compilation build script for htop-win.
Builds both x64 and ARM64 Windows executables from Linux/WSL.
"""

import os
import subprocess
import sys
import shutil
import multiprocessing
from pathlib import Path

# Build configuration
PROJECT_DIR = Path(__file__).parent.resolve()
TARGET_DIR = PROJECT_DIR / "target"
OUTPUT_DIR = Path("/mnt/c/code")  # Always output to Windows drive

# Cross-compilation toolchain paths
LLVM_MINGW = Path("/root/toolchains/llvm-mingw/bin")

# Get CPU count for parallel builds
CPU_COUNT = multiprocessing.cpu_count()

TARGETS = {
    "x64": {
        "triple": "x86_64-pc-windows-gnu",
        "cc": "x86_64-w64-mingw32-gcc",
        "ar": "x86_64-w64-mingw32-ar",
        "linker": "x86_64-w64-mingw32-gcc",
        "use_llvm_mingw": False,
    },
    "arm64": {
        "triple": "aarch64-pc-windows-gnullvm",
        "cc": "aarch64-w64-mingw32-clang",
        "ar": "llvm-ar",
        "linker": "aarch64-w64-mingw32-clang",
        "use_llvm_mingw": True,
    },
}


def get_env_for_target(target_name: str) -> dict:
    """Get environment variables for cross-compilation."""
    target = TARGETS[target_name]
    triple = target["triple"]
    env_triple = triple.replace("-", "_")

    env = os.environ.copy()

    # Add llvm-mingw to PATH if needed
    if target["use_llvm_mingw"]:
        env["PATH"] = f"{LLVM_MINGW}:{env['PATH']}"

    # Set C compiler and archiver
    env[f"CC_{env_triple}"] = target["cc"]
    env[f"AR_{env_triple}"] = target["ar"]

    # Set linker via cargo env var
    env[f"CARGO_TARGET_{env_triple.upper()}_LINKER"] = target["linker"]

    # For gnullvm targets, statically link the C runtime to avoid libunwind.dll dependency
    if "gnullvm" in triple:
        env[f"CARGO_TARGET_{env_triple.upper()}_RUSTFLAGS"] = "-C target-feature=+crt-static"

    return env


def run_command(cmd: list, env: dict = None, cwd: Path = None) -> bool:
    """Run a command and return success status."""
    print(f"\n>>> Running: {' '.join(cmd)}")
    try:
        subprocess.run(
            cmd,
            env=env or os.environ,
            cwd=cwd or PROJECT_DIR,
            check=True,
        )
        return True
    except subprocess.CalledProcessError as e:
        print(f"Error: Command failed with exit code {e.returncode}")
        return False


def check_prerequisites() -> bool:
    """Check that required tools are available."""
    print("Checking prerequisites...")

    # Check cargo
    if shutil.which("cargo") is None:
        print("Error: cargo not found")
        return False

    # Check x64 mingw
    if shutil.which("x86_64-w64-mingw32-gcc") is None:
        print("Error: x86_64-w64-mingw32-gcc not found")
        print("Install with: apt install gcc-mingw-w64-x86-64")
        return False

    # Check llvm-mingw for ARM64
    aarch64_clang = LLVM_MINGW / "aarch64-w64-mingw32-clang"
    if not aarch64_clang.exists():
        print(f"Error: llvm-mingw not found at {LLVM_MINGW}")
        print("ARM64 builds will not be available")
        return False

    print("All prerequisites met.")
    return True


def ensure_targets() -> bool:
    """Ensure Rust targets are installed."""
    print("\nEnsuring Rust targets are installed...")

    for name, target in TARGETS.items():
        cmd = ["rustup", "target", "add", target["triple"]]
        if not run_command(cmd):
            print(f"Failed to add target {target['triple']}")
            return False

    return True


def get_build_output_path(target_name: str) -> Path:
    """Get the path to the built executable in the target directory."""
    target = TARGETS[target_name]
    triple = target["triple"]
    return TARGET_DIR / triple / "release" / "htop-win.exe"


def get_final_output_path(target_name: str) -> Path:
    """Get the final output path on Windows drive."""
    return OUTPUT_DIR / f"htop-win-{target_name}.exe"


def copy_to_output(target_name: str) -> Path | None:
    """Copy the built executable to the output directory."""
    build_path = get_build_output_path(target_name)
    final_path = get_final_output_path(target_name)

    if not build_path.exists():
        print(f"Error: Built executable not found at {build_path}")
        return None

    # Ensure output directory exists
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    # Copy the file
    print(f"\n>>> Copying {build_path} -> {final_path}")
    shutil.copy2(build_path, final_path)

    return final_path


def build_target(target_name: str, jobs: int = None) -> bool:
    """Build for a specific target.

    Args:
        target_name: The target architecture name (x64, arm64)
        jobs: Number of parallel jobs (defaults to CPU count)
    """
    target = TARGETS[target_name]
    triple = target["triple"]
    num_jobs = jobs if jobs is not None else CPU_COUNT

    print(f"\n{'='*60}")
    print(f"Building for {target_name} ({triple})")
    print(f"Using {num_jobs} parallel jobs")
    print(f"{'='*60}")

    env = get_env_for_target(target_name)
    env["CARGO_BUILD_JOBS"] = str(num_jobs)

    cmd = ["cargo", "build", "--release", "--target", triple, "-j", str(num_jobs)]

    return run_command(cmd, env=env, cwd=PROJECT_DIR)


def check_target(target_name: str) -> bool:
    """Check (compile without linking) for a specific target."""
    target = TARGETS[target_name]
    triple = target["triple"]

    print(f"\n{'='*60}")
    print(f"Checking {target_name} ({triple})")
    print(f"{'='*60}")

    env = get_env_for_target(target_name)
    cmd = ["cargo", "check", "--target", triple]

    return run_command(cmd, env=env, cwd=PROJECT_DIR)


def get_system_memory_gb() -> float:
    """Get total system memory in GB."""
    try:
        with open("/proc/meminfo") as f:
            for line in f:
                if line.startswith("MemTotal:"):
                    kb = int(line.split()[1])
                    return kb / (1024 * 1024)
    except:
        pass
    return 0


def main():
    """Main entry point."""
    import argparse

    parser = argparse.ArgumentParser(
        description="Cross-compile htop-win for Windows"
    )
    parser.add_argument(
        "action",
        nargs="?",
        choices=["check", "build", "build-all"],
        default="build-all",
        help="Action to perform (default: build-all)"
    )
    parser.add_argument(
        "--target",
        choices=["x64", "arm64"],
        default="x64",
        help="Target architecture for single build (default: x64)"
    )
    parser.add_argument(
        "-j", "--jobs",
        type=int,
        default=None,
        help=f"Number of parallel jobs (default: {CPU_COUNT} - all CPUs)"
    )

    args = parser.parse_args()
    jobs = args.jobs

    # Display system info
    mem_gb = get_system_memory_gb()
    print(f"\n{'='*60}")
    print("htop-win Cross-Compilation Build")
    print(f"{'='*60}")
    print(f"  CPU cores: {CPU_COUNT}")
    if mem_gb > 0:
        print(f"  Total RAM: {mem_gb:.1f} GB")
    print(f"  Parallel jobs: {jobs if jobs else CPU_COUNT}")
    print(f"  Output directory: {OUTPUT_DIR}")

    # Check prerequisites
    if not check_prerequisites():
        sys.exit(1)

    if args.action == "check":
        if not ensure_targets():
            sys.exit(1)
        if not check_target(args.target):
            sys.exit(1)
        print(f"\n{args.target} check passed!")

    elif args.action == "build":
        if not ensure_targets():
            sys.exit(1)
        if not build_target(args.target, jobs):
            sys.exit(1)

        output = copy_to_output(args.target)
        if output:
            print(f"\nBuild successful!")
            print(f"  Output: {output}")
        else:
            build_path = get_build_output_path(args.target)
            print(f"\nBuild completed but output not found at {build_path}")
            sys.exit(1)

    elif args.action == "build-all":
        if not ensure_targets():
            sys.exit(1)

        results = {}
        for target_name in TARGETS:
            results[target_name] = build_target(target_name, jobs)

        print(f"\n{'='*60}")
        print("Build Summary")
        print(f"{'='*60}")

        all_success = True
        for target_name, success in results.items():
            if success:
                output = copy_to_output(target_name)
                if output:
                    size_mb = output.stat().st_size / (1024 * 1024)
                    print(f"  {target_name}: {output} ({size_mb:.2f} MB)")
                else:
                    print(f"  {target_name}: FAILED to copy")
                    all_success = False
            else:
                print(f"  {target_name}: FAILED to build")
                all_success = False

        if not all_success:
            sys.exit(1)

    print("\nDone!")


if __name__ == "__main__":
    main()
