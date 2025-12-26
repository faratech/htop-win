#!/usr/bin/env python3
"""
Version bumping script for htop-win.
Updates version in Cargo.toml and media/htop.rc
"""

import re
import sys
import argparse
from pathlib import Path

PROJECT_DIR = Path(__file__).parent.resolve()
CARGO_TOML = PROJECT_DIR / "Cargo.toml"
RESOURCE_FILE = PROJECT_DIR / "media" / "htop.rc"


def parse_version(version_str: str) -> tuple[int, int, int]:
    """Parse version string into tuple of (major, minor, patch)."""
    match = re.match(r"(\d+)\.(\d+)\.(\d+)", version_str)
    if not match:
        raise ValueError(f"Invalid version format: {version_str}")
    return int(match.group(1)), int(match.group(2)), int(match.group(3))


def get_current_version() -> str:
    """Get current version from Cargo.toml."""
    content = CARGO_TOML.read_text()
    match = re.search(r'^version\s*=\s*"([^"]+)"', content, re.MULTILINE)
    if not match:
        raise ValueError("Could not find version in Cargo.toml")
    return match.group(1)


def bump_version(current: str, bump_type: str) -> str:
    """Bump version based on type (major, minor, patch)."""
    major, minor, patch = parse_version(current)

    if bump_type == "major":
        return f"{major + 1}.0.0"
    elif bump_type == "minor":
        return f"{major}.{minor + 1}.0"
    elif bump_type == "patch":
        return f"{major}.{minor}.{patch + 1}"
    else:
        raise ValueError(f"Invalid bump type: {bump_type}")


def update_cargo_toml(old_version: str, new_version: str) -> bool:
    """Update version in Cargo.toml."""
    content = CARGO_TOML.read_text()
    new_content = re.sub(
        r'^(version\s*=\s*)"[^"]+"',
        f'\\1"{new_version}"',
        content,
        count=1,
        flags=re.MULTILINE
    )

    if content == new_content:
        print(f"  Warning: Cargo.toml was not modified")
        return False

    CARGO_TOML.write_text(new_content)
    print(f"  Updated Cargo.toml")
    return True


def update_resource_file(old_version: str, new_version: str) -> bool:
    """Update version in htop.rc resource file."""
    if not RESOURCE_FILE.exists():
        print(f"  Warning: {RESOURCE_FILE} not found, skipping")
        return False

    content = RESOURCE_FILE.read_text()
    major, minor, patch = parse_version(new_version)

    # Update FILEVERSION and PRODUCTVERSION (comma-separated)
    new_content = re.sub(
        r'(FILEVERSION\s+)\d+,\d+,\d+,\d+',
        f'\\g<1>{major},{minor},{patch},0',
        content
    )
    new_content = re.sub(
        r'(PRODUCTVERSION\s+)\d+,\d+,\d+,\d+',
        f'\\g<1>{major},{minor},{patch},0',
        new_content
    )

    # Update string versions
    new_content = re.sub(
        r'(VALUE\s+"FileVersion",\s*)"[^"]+"',
        f'\\1"{new_version}"',
        new_content
    )
    new_content = re.sub(
        r'(VALUE\s+"ProductVersion",\s*)"[^"]+"',
        f'\\1"{new_version}"',
        new_content
    )

    if content == new_content:
        print(f"  Warning: {RESOURCE_FILE.name} was not modified")
        return False

    RESOURCE_FILE.write_text(new_content)
    print(f"  Updated {RESOURCE_FILE.name}")
    return True


def main():
    parser = argparse.ArgumentParser(
        description="Bump htop-win version number"
    )
    parser.add_argument(
        "bump_type",
        nargs="?",
        choices=["major", "minor", "patch"],
        default="patch",
        help="Version component to bump (default: patch)"
    )
    parser.add_argument(
        "--set",
        metavar="VERSION",
        help="Set specific version (e.g., --set 1.0.0)"
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Show what would be changed without modifying files"
    )

    args = parser.parse_args()

    current_version = get_current_version()

    if args.set:
        # Validate the provided version
        try:
            parse_version(args.set)
        except ValueError as e:
            print(f"Error: {e}")
            sys.exit(1)
        new_version = args.set
    else:
        new_version = bump_version(current_version, args.bump_type)

    print(f"Version: {current_version} -> {new_version}")

    if args.dry_run:
        print("\n(dry-run, no files modified)")
        return

    print()
    update_cargo_toml(current_version, new_version)
    update_resource_file(current_version, new_version)

    print(f"\nDone! Version bumped to {new_version}")
    print("\nNext steps:")
    print(f"  1. Review changes: git diff")
    print(f"  2. Commit: git commit -am 'Bump version to {new_version}'")
    print(f"  3. Tag: git tag -a v{new_version} -m 'Release {new_version}'")
    print(f"  4. Push: git push && git push --tags")


if __name__ == "__main__":
    main()
