//! Visual regression testing for htop-win
//!
//! This module provides tools to capture terminal output and compare
//! against htop reference screenshots.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// ANSI color codes used by htop
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HtopColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

/// A single cell in the terminal
#[derive(Debug, Clone)]
pub struct Cell {
    pub char: char,
    pub fg: HtopColor,
    pub bg: HtopColor,
    pub bold: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            char: ' ',
            fg: HtopColor::White,
            bg: HtopColor::Black,
            bold: false,
        }
    }
}

/// A captured terminal screen
#[derive(Debug, Clone)]
pub struct Screen {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Vec<Cell>>,
}

impl Screen {
    pub fn new(width: usize, height: usize) -> Self {
        let cells = vec![vec![Cell::default(); width]; height];
        Self { width, height, cells }
    }

    /// Convert to plain text (no colors)
    pub fn to_text(&self) -> String {
        self.cells
            .iter()
            .map(|row| row.iter().map(|c| c.char).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Compare two screens and return a diff report
    pub fn diff(&self, other: &Screen) -> ScreenDiff {
        let mut diff = ScreenDiff::new();

        if self.width != other.width || self.height != other.height {
            diff.size_mismatch = Some((
                (self.width, self.height),
                (other.width, other.height),
            ));
            return diff;
        }

        for y in 0..self.height {
            for x in 0..self.width {
                let a = &self.cells[y][x];
                let b = &other.cells[y][x];

                if a.char != b.char {
                    diff.char_diffs.push(CharDiff {
                        x,
                        y,
                        expected: a.char,
                        actual: b.char,
                    });
                }

                if a.fg != b.fg || a.bg != b.bg {
                    diff.color_diffs.push(ColorDiff {
                        x,
                        y,
                        expected_fg: a.fg,
                        expected_bg: a.bg,
                        actual_fg: b.fg,
                        actual_bg: b.bg,
                    });
                }
            }
        }

        diff
    }
}

/// Difference between two screens
#[derive(Debug)]
pub struct ScreenDiff {
    pub size_mismatch: Option<((usize, usize), (usize, usize))>,
    pub char_diffs: Vec<CharDiff>,
    pub color_diffs: Vec<ColorDiff>,
}

impl ScreenDiff {
    pub fn new() -> Self {
        Self {
            size_mismatch: None,
            char_diffs: Vec::new(),
            color_diffs: Vec::new(),
        }
    }

    pub fn is_identical(&self) -> bool {
        self.size_mismatch.is_none()
            && self.char_diffs.is_empty()
            && self.color_diffs.is_empty()
    }

    /// Calculate similarity percentage (characters only)
    pub fn similarity(&self, total_cells: usize) -> f64 {
        if self.size_mismatch.is_some() {
            return 0.0;
        }
        let diff_count = self.char_diffs.len();
        ((total_cells - diff_count) as f64 / total_cells as f64) * 100.0
    }
}

#[derive(Debug)]
pub struct CharDiff {
    pub x: usize,
    pub y: usize,
    pub expected: char,
    pub actual: char,
}

#[derive(Debug)]
pub struct ColorDiff {
    pub x: usize,
    pub y: usize,
    pub expected_fg: HtopColor,
    pub expected_bg: HtopColor,
    pub actual_fg: HtopColor,
    pub actual_bg: HtopColor,
}

/// htop reference patterns for validation
pub struct HtopPatterns;

impl HtopPatterns {
    /// Validate CPU bar format
    pub fn validate_cpu_bar(line: &str) -> Result<(), String> {
        // Pattern: "  N[||||...    XX.X%]" where N is CPU number
        if !line.contains('[') || !line.contains(']') {
            return Err("CPU bar must contain [ and ]".to_string());
        }

        if !line.contains('%') {
            return Err("CPU bar must show percentage".to_string());
        }

        // Check for valid bar characters
        let bar_content: String = line
            .chars()
            .skip_while(|c| *c != '[')
            .skip(1)
            .take_while(|c| *c != ']')
            .filter(|c| *c != '|' && *c != ' ' && !c.is_numeric() && *c != '.' && *c != '%')
            .collect();

        if !bar_content.is_empty() {
            return Err(format!("Invalid characters in bar: {}", bar_content));
        }

        Ok(())
    }

    /// Validate memory bar format
    pub fn validate_memory_bar(line: &str) -> Result<(), String> {
        if !line.starts_with("Mem[") && !line.starts_with("Swp[") {
            return Err("Memory bar must start with 'Mem[' or 'Swp['".to_string());
        }

        if !line.contains('/') {
            return Err("Memory bar must show used/total format".to_string());
        }

        Ok(())
    }

    /// Validate process list header
    pub fn validate_header(line: &str) -> Result<(), String> {
        let required_columns = ["PID", "CPU", "MEM", "Command"];

        for col in required_columns {
            if !line.contains(col) {
                return Err(format!("Header missing column: {}", col));
            }
        }

        Ok(())
    }

    /// Validate function key bar
    pub fn validate_footer(line: &str) -> Result<(), String> {
        // Should contain F1-F10 labels
        let required = ["F1", "F10"];

        for key in required {
            if !line.contains(key) {
                return Err(format!("Footer missing: {}", key));
            }
        }

        Ok(())
    }
}

/// Test runner for visual tests
pub struct VisualTestRunner {
    snapshots_dir: String,
}

impl VisualTestRunner {
    pub fn new(snapshots_dir: &str) -> Self {
        Self {
            snapshots_dir: snapshots_dir.to_string(),
        }
    }

    /// Save a snapshot
    pub fn save_snapshot(&self, name: &str, content: &str) -> std::io::Result<()> {
        let path = Path::new(&self.snapshots_dir).join(format!("{}.txt", name));
        fs::create_dir_all(&self.snapshots_dir)?;
        fs::write(path, content)
    }

    /// Load a snapshot
    pub fn load_snapshot(&self, name: &str) -> std::io::Result<String> {
        let path = Path::new(&self.snapshots_dir).join(format!("{}.txt", name));
        fs::read_to_string(path)
    }

    /// Compare against snapshot
    pub fn compare_snapshot(&self, name: &str, actual: &str) -> Result<(), String> {
        match self.load_snapshot(name) {
            Ok(expected) => {
                if expected == actual {
                    Ok(())
                } else {
                    // Generate diff
                    let expected_lines: Vec<&str> = expected.lines().collect();
                    let actual_lines: Vec<&str> = actual.lines().collect();

                    let mut diff_report = String::new();
                    let max_lines = expected_lines.len().max(actual_lines.len());

                    for i in 0..max_lines {
                        let exp = expected_lines.get(i).unwrap_or(&"<missing>");
                        let act = actual_lines.get(i).unwrap_or(&"<missing>");

                        if exp != act {
                            diff_report.push_str(&format!(
                                "Line {}: expected '{}', got '{}'\n",
                                i + 1,
                                exp,
                                act
                            ));
                        }
                    }

                    Err(diff_report)
                }
            }
            Err(e) => Err(format!("Failed to load snapshot '{}': {}", name, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_bar_validation() {
        assert!(HtopPatterns::validate_cpu_bar("  0[||||||||            25.0%]").is_ok());
        assert!(HtopPatterns::validate_cpu_bar("  1[||||||||||||||||||||100.0%]").is_ok());
        assert!(HtopPatterns::validate_cpu_bar("  2[                      0.0%]").is_ok());

        // Invalid patterns
        assert!(HtopPatterns::validate_cpu_bar("invalid").is_err());
        assert!(HtopPatterns::validate_cpu_bar("no brackets").is_err());
    }

    #[test]
    fn test_memory_bar_validation() {
        assert!(HtopPatterns::validate_memory_bar("Mem[||||||||   1.5G/8.0G]").is_ok());
        assert!(HtopPatterns::validate_memory_bar("Swp[         0K/2.0G]").is_ok());

        assert!(HtopPatterns::validate_memory_bar("Memory: 50%").is_err());
    }

    #[test]
    fn test_header_validation() {
        assert!(HtopPatterns::validate_header(
            "  PID USER      PRI  NI  VIRT   RES   SHR S CPU% MEM%   TIME+  Command"
        )
        .is_ok());

        assert!(HtopPatterns::validate_header("Invalid header").is_err());
    }

    #[test]
    fn test_footer_validation() {
        assert!(HtopPatterns::validate_footer(
            "F1Help F2Setup F3Search F4Filter F5Tree F6SortBy F7Nice- F8Nice+ F9Kill F10Quit"
        )
        .is_ok());

        assert!(HtopPatterns::validate_footer("Press Q to quit").is_err());
    }

    #[test]
    fn test_screen_diff() {
        let mut screen1 = Screen::new(10, 2);
        screen1.cells[0][0].char = 'A';
        screen1.cells[0][1].char = 'B';

        let mut screen2 = Screen::new(10, 2);
        screen2.cells[0][0].char = 'A';
        screen2.cells[0][1].char = 'X'; // Different!

        let diff = screen1.diff(&screen2);
        assert!(!diff.is_identical());
        assert_eq!(diff.char_diffs.len(), 1);
        assert_eq!(diff.char_diffs[0].x, 1);
        assert_eq!(diff.char_diffs[0].y, 0);
        assert_eq!(diff.char_diffs[0].expected, 'B');
        assert_eq!(diff.char_diffs[0].actual, 'X');
    }

    #[test]
    fn test_similarity_calculation() {
        let screen1 = Screen::new(10, 10);
        let mut screen2 = Screen::new(10, 10);

        // Make 10 cells different (10% difference)
        for i in 0..10 {
            screen2.cells[0][i].char = 'X';
        }

        let diff = screen1.diff(&screen2);
        let similarity = diff.similarity(100);

        assert!((similarity - 90.0).abs() < 0.1, "Should be ~90% similar");
    }
}
