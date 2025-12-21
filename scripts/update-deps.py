#!/usr/bin/env python3
"""
Auto-update Cargo.toml dependencies to their latest versions.
Usage: python update-deps.py [--dry-run]
"""

import re
import subprocess
import sys
from pathlib import Path


def get_latest_version(crate_name: str) -> tuple[str | None, bool]:
    """
    Get the latest version of a crate from crates.io.
    Returns: (version, is_prerelease)
    """
    try:
        result = subprocess.run(
            ["cargo", "search", crate_name, "--limit", "1"],
            capture_output=True,
            text=True,
            timeout=30,
        )
        if result.returncode != 0:
            return None, False

        # Parse output like: crate_name = "1.2.3"    # description
        match = re.search(rf'^{re.escape(crate_name)}\s*=\s*"([^"]+)"', result.stdout, re.MULTILINE)
        if match:
            version = match.group(1)
            is_prerelease = any(pre in version.lower() for pre in ['alpha', 'beta', 'rc', '-'])
            return version, is_prerelease
    except (subprocess.TimeoutExpired, FileNotFoundError):
        pass
    return None, False


def version_matches(spec: str, version: str) -> bool:
    """Check if a version matches a version spec (e.g., "0.29" matches "0.29.0")."""
    spec_parts = spec.split('.')
    version_parts = version.split('.')

    for i, spec_part in enumerate(spec_parts):
        if i >= len(version_parts):
            return False
        if spec_part != version_parts[i]:
            return False
    return True


def parse_cargo_toml(content: str) -> dict[str, tuple[str, int, int]]:
    """
    Parse Cargo.toml and extract dependencies with their line positions.
    Returns: {crate_name: (current_version, line_start, line_end)}
    """
    deps = {}
    lines = content.split('\n')
    in_deps = False
    in_target_deps = False

    for i, line in enumerate(lines):
        # Check for dependency sections
        if re.match(r'^\[dependencies\]', line):
            in_deps = True
            in_target_deps = False
            continue
        elif re.match(r'^\[target\..*\.dependencies\]', line):
            in_deps = False
            in_target_deps = True
            continue
        elif re.match(r'^\[', line):
            in_deps = False
            in_target_deps = False
            continue

        if not (in_deps or in_target_deps):
            continue

        # Match simple dependency: crate = "version"
        simple_match = re.match(r'^(\w[\w-]*)\s*=\s*"([^"]+)"', line)
        if simple_match:
            crate_name = simple_match.group(1)
            version = simple_match.group(2)
            deps[crate_name] = (version, i, i)
            continue

        # Match table dependency: crate = { version = "x.y.z", ... }
        table_match = re.match(r'^(\w[\w-]*)\s*=\s*\{.*version\s*=\s*"([^"]+)"', line)
        if table_match:
            crate_name = table_match.group(1)
            version = table_match.group(2)
            deps[crate_name] = (version, i, i)
            continue

        # Match multi-line table dependency
        table_start = re.match(r'^(\w[\w-]*)\s*=\s*\{\s*$', line)
        if table_start:
            crate_name = table_start.group(1)
            # Look for version in subsequent lines
            for j in range(i + 1, min(i + 10, len(lines))):
                if '}' in lines[j]:
                    break
                version_match = re.search(r'version\s*=\s*"([^"]+)"', lines[j])
                if version_match:
                    deps[crate_name] = (version_match.group(1), i, j)
                    break

    return deps


def update_cargo_toml(content: str, updates: dict[str, str]) -> str:
    """Update Cargo.toml content with new versions."""
    lines = content.split('\n')

    for crate_name, new_version in updates.items():
        for i, line in enumerate(lines):
            # Update simple dependency
            if re.match(rf'^{re.escape(crate_name)}\s*=\s*"[^"]+"', line):
                lines[i] = re.sub(
                    rf'^({re.escape(crate_name)}\s*=\s*")[^"]+(")',
                    rf'\g<1>{new_version}\2',
                    line
                )
                continue

            # Update table dependency version
            if re.match(rf'^{re.escape(crate_name)}\s*=\s*\{{.*version\s*=\s*"[^"]+"', line):
                lines[i] = re.sub(
                    r'(version\s*=\s*")[^"]+(")',
                    rf'\g<1>{new_version}\2',
                    line
                )
                continue

            # Update multi-line table dependency
            if re.search(r'version\s*=\s*"[^"]+"', line):
                # Check if this line is part of the crate's definition
                for j in range(i - 1, max(i - 10, -1), -1):
                    if re.match(rf'^{re.escape(crate_name)}\s*=', lines[j]):
                        lines[i] = re.sub(
                            r'(version\s*=\s*")[^"]+(")',
                            rf'\g<1>{new_version}\2',
                            line
                        )
                        break
                    if re.match(r'^\[', lines[j]) or re.match(r'^\w', lines[j]):
                        break

    return '\n'.join(lines)


def main():
    dry_run = '--dry-run' in sys.argv

    # Find Cargo.toml
    cargo_toml = Path('Cargo.toml')
    if not cargo_toml.exists():
        print("Error: Cargo.toml not found in current directory")
        sys.exit(1)

    content = cargo_toml.read_text()
    deps = parse_cargo_toml(content)

    if not deps:
        print("No dependencies found in Cargo.toml")
        return

    print(f"Found {len(deps)} dependencies\n")
    print(f"{'Crate':<25} {'Current':<15} {'Latest':<15} {'Status'}")
    print("-" * 70)

    updates = {}
    for crate_name, (current_version, _, _) in sorted(deps.items()):
        latest, is_prerelease = get_latest_version(crate_name)

        if latest is None:
            status = "[!] Not found"
        elif latest == current_version or version_matches(current_version, latest):
            if is_prerelease:
                status = "[OK] Up to date (latest is prerelease)"
            else:
                status = "[OK] Up to date"
        elif is_prerelease:
            # Latest is prerelease, check if current matches the stable part
            stable_part = latest.split('-')[0]  # e.g., "0.30.0" from "0.30.0-beta.0"
            if version_matches(current_version, stable_part):
                status = "[OK] Up to date (latest is prerelease)"
            else:
                status = "[~] Prerelease available"
                # Don't auto-update to prereleases
        else:
            # Compare major versions to determine if it's a major update
            current_major = current_version.split('.')[0]
            latest_major = latest.split('.')[0]

            if current_major != latest_major:
                status = "[!!] Major update"
            else:
                status = "[*] Update available"
            updates[crate_name] = latest

        print(f"{crate_name:<25} {current_version:<15} {latest or 'N/A':<15} {status}")

    print()

    if not updates:
        print("All dependencies are up to date!")
        return

    if dry_run:
        print(f"\n[Dry run] Would update {len(updates)} dependencies")
        return

    # Ask for confirmation
    print(f"\nUpdate {len(updates)} dependencies? [y/N] ", end="")
    response = input().strip().lower()

    if response != 'y':
        print("Aborted")
        return

    # Perform updates
    new_content = update_cargo_toml(content, updates)
    cargo_toml.write_text(new_content)

    print(f"\n[OK] Updated {len(updates)} dependencies in Cargo.toml")
    print("Run 'cargo update' to update Cargo.lock")


if __name__ == "__main__":
    main()
