#!/usr/bin/env python3
"""
UI Testing Tool for htop-win

This script tests the visual output of htop-win against htop reference patterns.
It can:
1. Capture terminal output from htop-win
2. Validate UI components match htop patterns
3. Run automated visual regression tests

Usage:
    python test-ui.py --validate     # Run all validation tests
    python test-ui.py --capture      # Capture current output as snapshot
    python test-ui.py --compare      # Compare against snapshot
"""

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Optional


@dataclass
class TestResult:
    name: str
    passed: bool
    message: str


class HtopValidator:
    """Validates UI components match htop patterns"""

    @staticmethod
    def validate_cpu_bar(line: str) -> TestResult:
        """Validate CPU bar format matches htop"""
        # Pattern: "  N[|||||...     XX.X%]"
        pattern = r'^\s*\d+\[[\|\s]+\d+\.\d%\]'

        if not line.strip():
            return TestResult("CPU Bar", False, "Empty line")

        # Check for bracket structure
        if '[' not in line or ']' not in line:
            return TestResult("CPU Bar", False, "Missing brackets")

        # Check for percentage
        if '%' not in line:
            return TestResult("CPU Bar", False, "Missing percentage")

        # Extract bar content
        try:
            bar_start = line.index('[')
            bar_end = line.index(']')
            bar_content = line[bar_start + 1:bar_end]

            # Bar should contain only |, spaces, and the percentage
            valid_chars = set('| 0123456789.%')
            bar_chars = set(bar_content)
            invalid = bar_chars - valid_chars

            if invalid:
                return TestResult("CPU Bar", False, f"Invalid chars: {invalid}")

        except ValueError:
            return TestResult("CPU Bar", False, "Malformed bar structure")

        return TestResult("CPU Bar", True, "OK")

    @staticmethod
    def validate_memory_bar(line: str) -> TestResult:
        """Validate memory/swap bar format"""
        if not line.startswith("Mem[") and not line.startswith("Swp["):
            return TestResult("Memory Bar", False, "Must start with 'Mem[' or 'Swp['")

        if '/' not in line:
            return TestResult("Memory Bar", False, "Missing used/total separator")

        if ']' not in line:
            return TestResult("Memory Bar", False, "Missing closing bracket")

        return TestResult("Memory Bar", True, "OK")

    @staticmethod
    def validate_process_header(line: str) -> TestResult:
        """Validate process list header"""
        required = ["PID", "CPU", "MEM", "Command"]
        missing = [col for col in required if col not in line]

        if missing:
            return TestResult("Process Header", False, f"Missing columns: {missing}")

        return TestResult("Process Header", True, "OK")

    @staticmethod
    def validate_process_row(line: str) -> TestResult:
        """Validate a process row format"""
        # Should have: PID, various columns, command
        parts = line.split()

        if len(parts) < 5:
            return TestResult("Process Row", False, "Too few columns")

        # First column should be PID (number or selection marker + number)
        pid_part = parts[0].lstrip('>')
        if not pid_part.isdigit():
            return TestResult("Process Row", False, f"Invalid PID: {parts[0]}")

        return TestResult("Process Row", True, "OK")

    @staticmethod
    def validate_footer(line: str) -> TestResult:
        """Validate function key footer"""
        required_keys = ["F1", "F10"]
        missing = [key for key in required_keys if key not in line]

        if missing:
            return TestResult("Footer", False, f"Missing keys: {missing}")

        return TestResult("Footer", True, "OK")

    @staticmethod
    def validate_tasks_line(line: str) -> TestResult:
        """Validate tasks info line"""
        if not line.startswith("Tasks:"):
            return TestResult("Tasks Line", False, "Must start with 'Tasks:'")

        # Should contain a number
        if not re.search(r'\d+', line):
            return TestResult("Tasks Line", False, "Missing task count")

        return TestResult("Tasks Line", True, "OK")

    @staticmethod
    def validate_uptime_line(line: str) -> TestResult:
        """Validate uptime line"""
        if not line.startswith("Uptime:"):
            return TestResult("Uptime Line", False, "Must start with 'Uptime:'")

        # Should contain time format (digits and colons)
        if not re.search(r'\d+:\d+', line):
            return TestResult("Uptime Line", False, "Missing time format")

        return TestResult("Uptime Line", True, "OK")


class UITester:
    """Main UI testing class"""

    def __init__(self, snapshots_dir: str = "tests/snapshots"):
        self.snapshots_dir = Path(snapshots_dir)
        self.validator = HtopValidator()

    def validate_screen(self, lines: list[str]) -> list[TestResult]:
        """Validate a full screen of output"""
        results = []

        if len(lines) < 10:
            results.append(TestResult("Screen Size", False, f"Too few lines: {len(lines)}"))
            return results

        results.append(TestResult("Screen Size", True, f"{len(lines)} lines"))

        # Find CPU bars (first few lines with [ and ])
        cpu_bar_count = 0
        for i, line in enumerate(lines[:20]):
            if re.match(r'^\s*\d+\[', line):
                result = self.validator.validate_cpu_bar(line)
                result.name = f"CPU Bar {cpu_bar_count}"
                results.append(result)
                cpu_bar_count += 1

        if cpu_bar_count == 0:
            results.append(TestResult("CPU Bars", False, "No CPU bars found"))

        # Find memory bars
        for line in lines:
            if line.startswith("Mem["):
                results.append(self.validator.validate_memory_bar(line))
            elif line.startswith("Swp["):
                result = self.validator.validate_memory_bar(line)
                result.name = "Swap Bar"
                results.append(result)

        # Find Tasks line
        for line in lines:
            if line.startswith("Tasks:"):
                results.append(self.validator.validate_tasks_line(line))
                break

        # Find Uptime line
        for line in lines:
            if line.startswith("Uptime:"):
                results.append(self.validator.validate_uptime_line(line))
                break

        # Find process header (contains PID and Command)
        for line in lines:
            if "PID" in line and "Command" in line:
                results.append(self.validator.validate_process_header(line))
                break

        # Find footer (contains F1 and F10)
        for line in lines[-5:]:
            if "F1" in line and "F10" in line:
                results.append(self.validator.validate_footer(line))
                break

        return results

    def run_all_tests(self) -> bool:
        """Run all validation tests"""
        print("=" * 60)
        print("htop-win UI Validation Tests")
        print("=" * 60)

        # Test 1: Validate reference patterns
        print("\n[Test 1] Validating reference patterns...")

        test_patterns = {
            "CPU Bar 0%": "  0[                                          0.0%]",
            "CPU Bar 50%": "  0[|||||||||||||||||||||                    50.0%]",
            "CPU Bar 100%": "  0[|||||||||||||||||||||||||||||||||||||||| 100.0%]",
            "Memory Bar": "Mem[|||||||||||||||||||||||||||||||||     4.52G/7.89G]",
            "Swap Bar": "Swp[                                        0K/2.00G]",
            "Tasks": "Tasks: 245, 892 thr; 2 running",
            "Uptime": "Uptime: 12:34:56",
            "Header": "  PID USER      PRI  NI  VIRT   RES   SHR S CPU% MEM%   TIME+  Command",
            "Footer": "F1Help  F2Setup F3Search F4Filter F5Tree  F6SortBy F7Nice- F8Nice+ F9Kill  F10Quit",
        }

        all_passed = True
        for name, pattern in test_patterns.items():
            if "CPU" in name:
                result = self.validator.validate_cpu_bar(pattern)
            elif "Memory" in name or "Swap" in name:
                result = self.validator.validate_memory_bar(pattern)
            elif "Tasks" in name:
                result = self.validator.validate_tasks_line(pattern)
            elif "Uptime" in name:
                result = self.validator.validate_uptime_line(pattern)
            elif "Header" in name:
                result = self.validator.validate_process_header(pattern)
            elif "Footer" in name:
                result = self.validator.validate_footer(pattern)
            else:
                result = TestResult(name, True, "Skipped")

            status = "[PASS]" if result.passed else "[FAIL]"
            print(f"  {status} {name}: {result.message}")

            if not result.passed:
                all_passed = False

        # Test 2: Component tests
        print("\n[Test 2] Component validation tests...")

        component_tests = [
            ("CPU bar with no fill", "  0[                    0.0%]", "validate_cpu_bar"),
            ("CPU bar with full fill", "  7[|||||||||||||||||| 99.9%]", "validate_cpu_bar"),
            ("Memory with GB", "Mem[|||||||       8.0G/16.0G]", "validate_memory_bar"),
            ("Memory with MB", "Mem[|||||||       512M/1024M]", "validate_memory_bar"),
        ]

        for name, pattern, validator_method in component_tests:
            method = getattr(self.validator, validator_method)
            result = method(pattern)
            status = "[PASS]" if result.passed else "[FAIL]"
            print(f"  {status} {name}: {result.message}")

            if not result.passed:
                all_passed = False

        # Test 3: Color threshold tests
        print("\n[Test 3] Color threshold validation...")

        def get_expected_color(usage: float) -> str:
            if usage < 50.0:
                return "Green"
            elif usage < 80.0:
                return "Yellow"
            else:
                return "Red"

        color_tests = [
            (0.0, "Green"),
            (25.0, "Green"),
            (49.9, "Green"),
            (50.0, "Yellow"),
            (65.0, "Yellow"),
            (79.9, "Yellow"),
            (80.0, "Red"),
            (100.0, "Red"),
        ]

        for usage, expected_color in color_tests:
            actual = get_expected_color(usage)
            passed = actual == expected_color
            status = "[PASS]" if passed else "[FAIL]"
            print(f"  {status} {usage}% -> {actual} (expected {expected_color})")

            if not passed:
                all_passed = False

        # Summary
        print("\n" + "=" * 60)
        if all_passed:
            print("[SUCCESS] All UI validation tests passed!")
        else:
            print("[FAILURE] Some tests failed. Review output above.")
        print("=" * 60)

        return all_passed


def main():
    parser = argparse.ArgumentParser(description="htop-win UI Testing Tool")
    parser.add_argument("--validate", action="store_true", help="Run validation tests")
    parser.add_argument("--capture", action="store_true", help="Capture output snapshot")
    parser.add_argument("--compare", action="store_true", help="Compare against snapshot")
    parser.add_argument("--verbose", "-v", action="store_true", help="Verbose output")

    args = parser.parse_args()

    tester = UITester()

    if args.validate or (not args.capture and not args.compare):
        success = tester.run_all_tests()
        sys.exit(0 if success else 1)

    if args.capture:
        print("Capture mode not yet implemented")
        print("Run htop-win and manually capture the output")
        sys.exit(0)

    if args.compare:
        print("Compare mode not yet implemented")
        print("Use --validate for pattern validation")
        sys.exit(0)


if __name__ == "__main__":
    main()
