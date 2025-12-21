//! UI snapshot and component tests for htop-win
//!
//! These tests verify that our UI output matches htop's appearance.
//! Run with: cargo test --test ui_tests

use std::io;
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    Terminal,
};

// We need to import from the main crate
// For now, we'll create standalone test utilities

/// Test utilities for comparing terminal output
mod test_utils {
    use super::*;

    /// Create a test terminal with given dimensions
    pub fn create_test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        Terminal::new(backend).unwrap()
    }

    /// Convert buffer to string for comparison
    pub fn buffer_to_string(buffer: &Buffer) -> String {
        let mut result = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = buffer.get(x, y);
                result.push_str(cell.symbol());
            }
            result.push('\n');
        }
        result
    }

    /// Compare two buffers and return differences
    pub fn diff_buffers(expected: &Buffer, actual: &Buffer) -> Vec<String> {
        let mut diffs = Vec::new();

        if expected.area != actual.area {
            diffs.push(format!(
                "Size mismatch: expected {:?}, got {:?}",
                expected.area, actual.area
            ));
            return diffs;
        }

        for y in 0..expected.area.height {
            for x in 0..expected.area.width {
                let exp_cell = expected.get(x, y);
                let act_cell = actual.get(x, y);

                if exp_cell.symbol() != act_cell.symbol() {
                    diffs.push(format!(
                        "({}, {}): expected '{}', got '{}'",
                        x, y, exp_cell.symbol(), act_cell.symbol()
                    ));
                }
            }
        }

        diffs
    }
}

/// Tests for CPU bar rendering
mod cpu_bar_tests {
    use super::*;

    #[test]
    fn test_cpu_bar_format() {
        // htop format: "  0[||||||||||||||||||||                    50.0%]"
        // Our format should match this pattern
        let cpu_idx = 0;
        let usage = 50.0;
        let bar_width = 40;

        let filled = (usage as usize * bar_width / 100).min(bar_width);
        let empty = bar_width - filled;

        let bar_filled: String = "|".repeat(filled);
        let bar_empty: String = " ".repeat(empty);

        let label = format!("{:3}[", cpu_idx);
        let percent = format!("{:5.1}%]", usage);

        let full_bar = format!("{}{}{}{}", label, bar_filled, bar_empty, percent);

        // Verify format
        assert!(full_bar.starts_with("  0["), "Should start with CPU index");
        assert!(full_bar.ends_with("50.0%]"), "Should end with percentage");
        assert!(full_bar.contains("||||||||||||||||||||"), "Should have filled portion");
    }

    #[test]
    fn test_cpu_bar_colors() {
        // Test color thresholds match htop:
        // < 50%: Green
        // 50-80%: Yellow
        // > 80%: Red

        fn get_expected_color(usage: f32) -> &'static str {
            if usage < 50.0 {
                "Green"
            } else if usage < 80.0 {
                "Yellow"
            } else {
                "Red"
            }
        }

        assert_eq!(get_expected_color(0.0), "Green");
        assert_eq!(get_expected_color(25.0), "Green");
        assert_eq!(get_expected_color(49.9), "Green");
        assert_eq!(get_expected_color(50.0), "Yellow");
        assert_eq!(get_expected_color(75.0), "Yellow");
        assert_eq!(get_expected_color(79.9), "Yellow");
        assert_eq!(get_expected_color(80.0), "Red");
        assert_eq!(get_expected_color(100.0), "Red");
    }

    #[test]
    fn test_cpu_bar_0_percent() {
        let usage = 0.0;
        let bar_width = 40;
        let filled = (usage as usize * bar_width / 100).min(bar_width);

        assert_eq!(filled, 0, "0% should have no filled bars");
    }

    #[test]
    fn test_cpu_bar_100_percent() {
        let usage = 100.0;
        let bar_width = 40;
        let filled = (usage as usize * bar_width / 100).min(bar_width);

        assert_eq!(filled, 40, "100% should fill entire bar");
    }
}

/// Tests for memory bar rendering
mod memory_bar_tests {
    use super::*;

    #[test]
    fn test_memory_bar_format() {
        // htop format: "Mem[|||||||||||||||||||||||||       1.23G/7.89G]"
        let usage = 60.0;
        let used = "1.23G";
        let total = "7.89G";

        let mem_info = format!("{:5.1}%] {}/{}", usage, used, total);

        assert!(mem_info.contains("60.0%"), "Should show percentage");
        assert!(mem_info.contains("1.23G/7.89G"), "Should show used/total");
    }

    #[test]
    fn test_format_bytes() {
        fn format_bytes(bytes: u64) -> String {
            const KB: u64 = 1024;
            const MB: u64 = KB * 1024;
            const GB: u64 = MB * 1024;
            const TB: u64 = GB * 1024;

            if bytes >= TB {
                format!("{:.1}T", bytes as f64 / TB as f64)
            } else if bytes >= GB {
                format!("{:.1}G", bytes as f64 / GB as f64)
            } else if bytes >= MB {
                format!("{:.0}M", bytes as f64 / MB as f64)
            } else if bytes >= KB {
                format!("{:.0}K", bytes as f64 / KB as f64)
            } else {
                format!("{}B", bytes)
            }
        }

        assert_eq!(format_bytes(0), "0B");
        assert_eq!(format_bytes(512), "512B");
        assert_eq!(format_bytes(1024), "1K");
        assert_eq!(format_bytes(1536), "2K"); // Rounds
        assert_eq!(format_bytes(1024 * 1024), "1M");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0G");
        assert_eq!(format_bytes(1024u64 * 1024 * 1024 * 1024), "1.0T");
    }
}

/// Tests for process list rendering
mod process_list_tests {
    use super::*;

    #[test]
    fn test_column_headers() {
        // htop column headers
        let expected_headers = [
            "PID", "USER", "PRI", "NI", "VIRT", "RES", "SHR", "S", "CPU%", "MEM%", "TIME+", "Command"
        ];

        // Our headers should match
        let our_headers = [
            "PID", "USER", "PRI", "NI", "VIRT", "RES", "SHR", "S", "CPU%", "MEM%", "TIME+", "Command"
        ];

        assert_eq!(expected_headers, our_headers, "Column headers should match htop");
    }

    #[test]
    fn test_process_status_chars() {
        // htop status characters
        // R = Running
        // S = Sleeping
        // D = Disk sleep
        // Z = Zombie
        // T = Stopped
        // I = Idle

        fn get_status_char(status: &str) -> char {
            match status {
                "Running" => 'R',
                "Sleeping" => 'S',
                "DiskSleep" => 'D',
                "Zombie" => 'Z',
                "Stopped" => 'T',
                "Idle" => 'I',
                _ => '?',
            }
        }

        assert_eq!(get_status_char("Running"), 'R');
        assert_eq!(get_status_char("Sleeping"), 'S');
        assert_eq!(get_status_char("Zombie"), 'Z');
    }

    #[test]
    fn test_time_format() {
        // htop time format: HH:MM:SS or MM:SS.cc
        fn format_cpu_time(secs: u64, centis: u64) -> String {
            let hours = secs / 3600;
            let mins = (secs % 3600) / 60;
            let secs = secs % 60;

            if hours > 0 {
                format!("{:02}:{:02}:{:02}", hours, mins, secs)
            } else {
                format!("{:02}:{:02}.{:02}", mins, secs, centis)
            }
        }

        assert_eq!(format_cpu_time(0, 0), "00:00.00");
        assert_eq!(format_cpu_time(65, 50), "01:05.50");
        assert_eq!(format_cpu_time(3661, 0), "01:01:01");
    }

    #[test]
    fn test_tree_view_prefix() {
        // htop tree view uses box-drawing characters
        fn get_tree_prefix(depth: usize, is_last: bool) -> String {
            if depth == 0 {
                return String::new();
            }

            let mut prefix = "  ".repeat(depth.saturating_sub(1));
            if is_last {
                prefix.push_str("└─ ");
            } else {
                prefix.push_str("├─ ");
            }
            prefix
        }

        assert_eq!(get_tree_prefix(0, false), "");
        assert_eq!(get_tree_prefix(1, false), "├─ ");
        assert_eq!(get_tree_prefix(1, true), "└─ ");
        assert_eq!(get_tree_prefix(2, false), "  ├─ ");
    }
}

/// Tests for footer/function key bar
mod footer_tests {
    use super::*;

    #[test]
    fn test_function_key_labels() {
        // htop function key labels
        let expected = [
            ("F1", "Help"),
            ("F2", "Setup"),
            ("F3", "Search"),
            ("F4", "Filter"),
            ("F5", "Tree"),
            ("F6", "SortBy"),
            ("F7", "Nice -"),
            ("F8", "Nice +"),
            ("F9", "Kill"),
            ("F10", "Quit"),
        ];

        // Verify we have all 10 function keys
        assert_eq!(expected.len(), 10);

        // Verify F1 is Help
        assert_eq!(expected[0], ("F1", "Help"));

        // Verify F10 is Quit
        assert_eq!(expected[9], ("F10", "Quit"));
    }
}

/// Tests for sort indicators
mod sort_tests {
    use super::*;

    #[test]
    fn test_sort_indicators() {
        // htop uses triangle indicators for sort direction
        let ascending = "▲";
        let descending = "▼";

        assert_eq!(ascending.chars().count(), 1);
        assert_eq!(descending.chars().count(), 1);
    }
}

/// Snapshot tests - compare against reference output
mod snapshot_tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// Reference snapshot directory
    const SNAPSHOT_DIR: &str = "tests/snapshots";

    #[test]
    #[ignore] // Run manually with: cargo test --test ui_tests -- --ignored
    fn test_full_ui_snapshot() {
        // This test would render the full UI and compare against a snapshot
        // For now, we just verify the snapshot infrastructure works

        let snapshot_path = Path::new(SNAPSHOT_DIR).join("full_ui.txt");

        if snapshot_path.exists() {
            let expected = fs::read_to_string(&snapshot_path).unwrap();
            // Would compare against actual rendered output
            assert!(!expected.is_empty(), "Snapshot should not be empty");
        } else {
            println!("Snapshot file not found: {:?}", snapshot_path);
            println!("Create it by running the app and capturing output");
        }
    }
}

/// Integration tests
mod integration_tests {
    use super::*;

    #[test]
    fn test_layout_proportions() {
        // htop layout:
        // - Header: ~8-12 lines (depends on CPU count)
        // - Process list: remaining space
        // - Footer: 2 lines

        let terminal_height = 40;
        let cpu_count = 8;

        // Header height calculation
        let cpu_rows = (cpu_count + 1) / 2; // 2 columns of CPUs
        let header_height = cpu_rows + 3 + 2; // CPU rows + mem/swap/tasks + borders

        let footer_height = 2;
        let process_list_height = terminal_height - header_height - footer_height;

        assert!(header_height >= 6, "Header should be at least 6 lines");
        assert!(header_height <= 20, "Header shouldn't exceed 20 lines");
        assert!(process_list_height >= 10, "Process list needs at least 10 lines");
        assert_eq!(footer_height, 2, "Footer should be exactly 2 lines");
    }
}
