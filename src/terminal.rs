//! Minimal terminal UI library - replaces ratatui for smaller binary size
//!
//! Provides: Buffer, Terminal, Frame, widgets (Block, Paragraph, Table, List, etc.)

#![allow(dead_code)] // Library provides full API even if not all used

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    style::{
        Attribute, Color as CtColor, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{self, Clear as CtClear, ClearType},
    ExecutableCommand, QueueableCommand,
};
use std::io::{self, Stdout, Write};

// ============================================================================
// Layout types
// ============================================================================

/// Rectangle with position and size
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self { x, y, width, height }
    }

    pub fn area(&self) -> u16 {
        self.width.saturating_mul(self.height)
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn left(&self) -> u16 {
        self.x
    }

    pub fn right(&self) -> u16 {
        self.x.saturating_add(self.width)
    }

    pub fn top(&self) -> u16 {
        self.y
    }

    pub fn bottom(&self) -> u16 {
        self.y.saturating_add(self.height)
    }

    /// Create inner rect with margin
    pub fn inner(&self, margin: u16) -> Rect {
        Rect {
            x: self.x.saturating_add(margin),
            y: self.y.saturating_add(margin),
            width: self.width.saturating_sub(margin * 2),
            height: self.height.saturating_sub(margin * 2),
        }
    }
}

/// Layout constraint
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constraint {
    Percentage(u16),
    Length(u16),
    Min(u16),
    Max(u16),
    Ratio(u32, u32),
    Fill(u16),
}

/// Layout direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Horizontal,
    Vertical,
}

/// Simple layout calculator (no cassowary needed)
#[derive(Debug, Clone)]
pub struct Layout {
    direction: Direction,
    constraints: Vec<Constraint>,
    margin: u16,
    spacing: u16,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            direction: Direction::Vertical,
            constraints: Vec::new(),
            margin: 0,
            spacing: 0,
        }
    }
}

impl Layout {
    pub fn horizontal(constraints: impl Into<Vec<Constraint>>) -> Self {
        Self {
            direction: Direction::Horizontal,
            constraints: constraints.into(),
            margin: 0,
            spacing: 0,
        }
    }

    pub fn vertical(constraints: impl Into<Vec<Constraint>>) -> Self {
        Self {
            direction: Direction::Vertical,
            constraints: constraints.into(),
            margin: 0,
            spacing: 0,
        }
    }

    pub fn direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    pub fn constraints(mut self, constraints: impl Into<Vec<Constraint>>) -> Self {
        self.constraints = constraints.into();
        self
    }

    pub fn margin(mut self, margin: u16) -> Self {
        self.margin = margin;
        self
    }

    pub fn spacing(mut self, spacing: u16) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn split(&self, area: Rect) -> Vec<Rect> {
        let area = area.inner(self.margin);
        if self.constraints.is_empty() || area.is_empty() {
            return vec![area];
        }

        // Account for spacing between elements
        let spacing_total = self.spacing * (self.constraints.len().saturating_sub(1)) as u16;
        let total = match self.direction {
            Direction::Horizontal => area.width.saturating_sub(spacing_total) as i32,
            Direction::Vertical => area.height.saturating_sub(spacing_total) as i32,
        };

        let mut sizes: Vec<i32> = vec![0; self.constraints.len()];
        let mut remaining = total;
        let mut flex_count = 0;
        let mut min_values: Vec<i32> = vec![0; self.constraints.len()];

        // First pass: fixed sizes (Length, Percentage, Ratio)
        // Min and Fill are flexible - they start at minimum and can grow
        for (i, constraint) in self.constraints.iter().enumerate() {
            match constraint {
                Constraint::Length(len) => {
                    sizes[i] = (*len as i32).min(remaining);
                    remaining -= sizes[i];
                }
                Constraint::Percentage(pct) => {
                    sizes[i] = (total * (*pct as i32) / 100).min(remaining);
                    remaining -= sizes[i];
                }
                Constraint::Ratio(num, den) => {
                    if *den > 0 {
                        sizes[i] = (total * (*num as i32) / (*den as i32)).min(remaining);
                        remaining -= sizes[i];
                    }
                }
                Constraint::Min(min) => {
                    // Reserve minimum, but track as flexible
                    min_values[i] = *min as i32;
                    sizes[i] = (*min as i32).min(remaining);
                    remaining -= sizes[i];
                    flex_count += 1;
                }
                Constraint::Max(max) => {
                    sizes[i] = (*max as i32).min(remaining);
                    remaining -= sizes[i];
                }
                Constraint::Fill(_) => {
                    flex_count += 1;
                }
            }
        }

        // Second pass: distribute remaining to flexible constraints (Min and Fill)
        if flex_count > 0 && remaining > 0 {
            let per_flex = remaining / flex_count;
            for (i, constraint) in self.constraints.iter().enumerate() {
                match constraint {
                    Constraint::Min(_) | Constraint::Fill(_) => {
                        sizes[i] += per_flex;
                    }
                    _ => {}
                }
            }
        }

        // Build rects with spacing
        let mut pos = match self.direction {
            Direction::Horizontal => area.x,
            Direction::Vertical => area.y,
        };

        sizes
            .iter()
            .enumerate()
            .map(|(i, &size)| {
                let size = size.max(0) as u16;
                let rect = match self.direction {
                    Direction::Horizontal => Rect::new(pos, area.y, size, area.height),
                    Direction::Vertical => Rect::new(area.x, pos, area.width, size),
                };
                pos += size;
                // Add spacing after each element except the last
                if i < self.constraints.len() - 1 {
                    pos += self.spacing;
                }
                rect
            })
            .collect()
    }
}

// ============================================================================
// Style types
// ============================================================================

/// Terminal colors
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    White,
    Rgb(u8, u8, u8),
    Indexed(u8),
}

impl Color {
    fn to_crossterm(self) -> CtColor {
        match self {
            Color::Reset => CtColor::Reset,
            Color::Black => CtColor::Black,
            Color::Red => CtColor::DarkRed,
            Color::Green => CtColor::DarkGreen,
            Color::Yellow => CtColor::DarkYellow,
            Color::Blue => CtColor::DarkBlue,
            Color::Magenta => CtColor::DarkMagenta,
            Color::Cyan => CtColor::DarkCyan,
            Color::Gray => CtColor::Grey,
            Color::DarkGray => CtColor::DarkGrey,
            Color::LightRed => CtColor::Red,
            Color::LightGreen => CtColor::Green,
            Color::LightYellow => CtColor::Yellow,
            Color::LightBlue => CtColor::Blue,
            Color::LightMagenta => CtColor::Magenta,
            Color::LightCyan => CtColor::Cyan,
            Color::White => CtColor::White,
            Color::Rgb(r, g, b) => CtColor::Rgb { r, g, b },
            Color::Indexed(i) => CtColor::AnsiValue(i),
        }
    }
}

bitflags::bitflags! {
    /// Text modifiers
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Modifier: u16 {
        const BOLD = 0b0000_0001;
        const DIM = 0b0000_0010;
        const ITALIC = 0b0000_0100;
        const UNDERLINED = 0b0000_1000;
        const SLOW_BLINK = 0b0001_0000;
        const RAPID_BLINK = 0b0010_0000;
        const REVERSED = 0b0100_0000;
        const HIDDEN = 0b1000_0000;
        const CROSSED_OUT = 0b0001_0000_0000;
    }
}

/// Combined style (fg, bg, modifiers)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub add_modifier: Modifier,
    pub sub_modifier: Modifier,
}

impl Style {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    pub fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    pub fn add_modifier(mut self, modifier: Modifier) -> Self {
        self.add_modifier |= modifier;
        self
    }

    pub fn remove_modifier(mut self, modifier: Modifier) -> Self {
        self.sub_modifier |= modifier;
        self
    }

    pub fn reset() -> Self {
        Self::default()
    }

    /// Patch this style with another (other takes precedence)
    pub fn patch(mut self, other: Style) -> Self {
        if other.fg.is_some() {
            self.fg = other.fg;
        }
        if other.bg.is_some() {
            self.bg = other.bg;
        }
        self.add_modifier |= other.add_modifier;
        self.sub_modifier |= other.sub_modifier;
        self.add_modifier &= !self.sub_modifier;
        self
    }
}

// ============================================================================
// Text types
// ============================================================================

/// Styled text segment
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span<'a> {
    pub content: std::borrow::Cow<'a, str>,
    pub style: Style,
}

impl<'a> Span<'a> {
    pub fn raw<T: Into<std::borrow::Cow<'a, str>>>(content: T) -> Self {
        Self {
            content: content.into(),
            style: Style::default(),
        }
    }

    pub fn styled<T: Into<std::borrow::Cow<'a, str>>>(content: T, style: Style) -> Self {
        Self {
            content: content.into(),
            style,
        }
    }

    pub fn width(&self) -> usize {
        unicode_width::UnicodeWidthStr::width(self.content.as_ref())
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl<'a> From<&'a str> for Span<'a> {
    fn from(s: &'a str) -> Self {
        Span::raw(s)
    }
}

impl<'a> From<String> for Span<'a> {
    fn from(s: String) -> Self {
        Span::raw(s)
    }
}

/// Line of spans
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Line<'a> {
    pub spans: Vec<Span<'a>>,
    pub style: Style,
}

impl<'a> Line<'a> {
    pub fn raw<T: Into<std::borrow::Cow<'a, str>>>(content: T) -> Self {
        Self {
            spans: vec![Span::raw(content)],
            style: Style::default(),
        }
    }

    pub fn styled<T: Into<std::borrow::Cow<'a, str>>>(content: T, style: Style) -> Self {
        Self {
            spans: vec![Span::styled(content, style)],
            style: Style::default(),
        }
    }

    pub fn width(&self) -> usize {
        self.spans.iter().map(|s| s.width()).sum()
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl<'a> From<&'a str> for Line<'a> {
    fn from(s: &'a str) -> Self {
        Line::raw(s)
    }
}

impl<'a> From<String> for Line<'a> {
    fn from(s: String) -> Self {
        Line::raw(s)
    }
}

impl<'a> From<Span<'a>> for Line<'a> {
    fn from(span: Span<'a>) -> Self {
        Self {
            spans: vec![span],
            style: Style::default(),
        }
    }
}

impl<'a> From<Vec<Span<'a>>> for Line<'a> {
    fn from(spans: Vec<Span<'a>>) -> Self {
        Self {
            spans,
            style: Style::default(),
        }
    }
}

/// Multi-line text
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Text<'a> {
    pub lines: Vec<Line<'a>>,
}

impl<'a> Text<'a> {
    pub fn raw<T: Into<std::borrow::Cow<'a, str>>>(content: T) -> Self {
        let content = content.into();
        let lines = content.lines().map(|l| Line::raw(l.to_string())).collect();
        Self { lines }
    }
}

impl<'a> From<&'a str> for Text<'a> {
    fn from(s: &'a str) -> Self {
        Text::raw(s)
    }
}

impl<'a> From<String> for Text<'a> {
    fn from(s: String) -> Self {
        Text::raw(s)
    }
}

impl<'a> From<Line<'a>> for Text<'a> {
    fn from(line: Line<'a>) -> Self {
        Self { lines: vec![line] }
    }
}

impl<'a> From<Vec<Line<'a>>> for Text<'a> {
    fn from(lines: Vec<Line<'a>>) -> Self {
        Self { lines }
    }
}

impl<'a> From<Span<'a>> for Text<'a> {
    fn from(span: Span<'a>) -> Self {
        Self {
            lines: vec![Line::from(span)],
        }
    }
}

// ============================================================================
// Buffer and Cell
// ============================================================================

/// Single cell in the buffer (internal type, not exported as Cell)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferCell {
    pub symbol: String,
    pub fg: Color,
    pub bg: Color,
    pub modifier: Modifier,
    /// True if this cell is a continuation of a wide character in the previous cell
    pub is_continuation: bool,
}

impl Default for BufferCell {
    fn default() -> Self {
        Self {
            symbol: " ".to_string(),
            fg: Color::Reset,
            bg: Color::Reset,
            modifier: Modifier::empty(),
            is_continuation: false,
        }
    }
}

impl BufferCell {
    pub fn set_symbol(&mut self, symbol: &str) -> &mut Self {
        self.symbol.clear();
        self.symbol.push_str(symbol);
        self
    }

    pub fn set_char(&mut self, ch: char) -> &mut Self {
        self.symbol.clear();
        self.symbol.push(ch);
        self
    }

    pub fn set_style(&mut self, style: Style) -> &mut Self {
        if let Some(fg) = style.fg {
            self.fg = fg;
        }
        if let Some(bg) = style.bg {
            self.bg = bg;
        }
        self.modifier |= style.add_modifier;
        self.modifier &= !style.sub_modifier;
        self
    }

    pub fn reset(&mut self) {
        self.symbol.clear();
        self.symbol.push(' ');
        self.fg = Color::Reset;
        self.bg = Color::Reset;
        self.modifier = Modifier::empty();
        self.is_continuation = false;
    }

    /// Mark this cell as a continuation of a wide character
    pub fn set_continuation(&mut self) -> &mut Self {
        self.symbol.clear();
        self.is_continuation = true;
        self
    }
}

/// 2D buffer of cells
#[derive(Debug, Clone, Default)]
pub struct Buffer {
    pub area: Rect,
    pub content: Vec<BufferCell>,
}

impl Buffer {
    pub fn empty(area: Rect) -> Self {
        let size = area.area() as usize;
        Self {
            area,
            content: vec![BufferCell::default(); size],
        }
    }

    pub fn filled(area: Rect, cell: BufferCell) -> Self {
        let size = area.area() as usize;
        Self {
            area,
            content: vec![cell; size],
        }
    }

    fn index_of(&self, x: u16, y: u16) -> usize {
        let x = x.saturating_sub(self.area.x);
        let y = y.saturating_sub(self.area.y);
        (y as usize) * (self.area.width as usize) + (x as usize)
    }

    pub fn get_mut(&mut self, x: u16, y: u16) -> Option<&mut BufferCell> {
        if x >= self.area.x
            && x < self.area.x + self.area.width
            && y >= self.area.y
            && y < self.area.y + self.area.height
        {
            let idx = self.index_of(x, y);
            self.content.get_mut(idx)
        } else {
            None
        }
    }

    pub fn get(&self, x: u16, y: u16) -> Option<&BufferCell> {
        if x >= self.area.x
            && x < self.area.x + self.area.width
            && y >= self.area.y
            && y < self.area.y + self.area.height
        {
            let idx = self.index_of(x, y);
            self.content.get(idx)
        } else {
            None
        }
    }

    pub fn set_string(&mut self, x: u16, y: u16, string: &str, style: Style) {
        self.set_string_truncated(x, y, string, u16::MAX, style);
    }

    pub fn set_string_truncated(&mut self, x: u16, y: u16, string: &str, max_width: u16, style: Style) {
        let mut col = x;
        let max_col = x.saturating_add(max_width).min(self.area.x + self.area.width);

        for ch in string.chars() {
            let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
            if col + width > max_col {
                break;
            }
            if let Some(cell) = self.get_mut(col, y) {
                cell.set_char(ch);
                cell.set_style(style);
                cell.is_continuation = false;
            }
            // Mark continuation cells for wide characters
            for i in 1..width {
                if let Some(cont_cell) = self.get_mut(col + i, y) {
                    cont_cell.set_continuation();
                    cont_cell.set_style(style);
                }
            }
            col += width.max(1);
        }
    }

    pub fn set_line(&mut self, x: u16, y: u16, line: &Line<'_>, max_width: u16) {
        let mut col = x;
        let max_col = x.saturating_add(max_width).min(self.area.x + self.area.width);

        for span in &line.spans {
            let style = line.style.patch(span.style);
            for ch in span.content.chars() {
                let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
                if col + width > max_col {
                    return;
                }
                if let Some(cell) = self.get_mut(col, y) {
                    cell.set_char(ch);
                    cell.set_style(style);
                    cell.is_continuation = false; // This cell has actual content
                }
                // Mark continuation cells for wide characters (width > 1)
                // These cells are "occupied" by the wide char but contain no content
                for i in 1..width {
                    if let Some(cont_cell) = self.get_mut(col + i, y) {
                        cont_cell.set_continuation();
                        cont_cell.set_style(style); // Keep same style for background
                    }
                }
                col += width.max(1);
            }
        }
    }

    pub fn set_span(&mut self, x: u16, y: u16, span: &Span<'_>, max_width: u16) {
        let line = Line::from(span.clone());
        self.set_line(x, y, &line, max_width);
    }

    pub fn set_style(&mut self, area: Rect, style: Style) {
        for y in area.y..area.y.saturating_add(area.height) {
            for x in area.x..area.x.saturating_add(area.width) {
                if let Some(cell) = self.get_mut(x, y) {
                    cell.set_style(style);
                }
            }
        }
    }
}

// ============================================================================
// Terminal and Frame
// ============================================================================

/// Crossterm backend
pub struct CrosstermBackend {
    stdout: Stdout,
}

impl CrosstermBackend {
    pub fn new(stdout: Stdout) -> Self {
        Self { stdout }
    }
}

impl io::Write for CrosstermBackend {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stdout.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stdout.flush()
    }
}

/// Terminal wrapper
pub struct Terminal {
    backend: CrosstermBackend,
    buffers: [Buffer; 2],
    current: usize,
    hidden_cursor: bool,
}

impl Terminal {
    pub fn new(backend: CrosstermBackend) -> io::Result<Self> {
        let size = terminal::size()?;
        let area = Rect::new(0, 0, size.0, size.1);
        Ok(Self {
            backend,
            buffers: [Buffer::empty(area), Buffer::empty(area)],
            current: 0,
            hidden_cursor: false,
        })
    }

    pub fn draw<F>(&mut self, f: F) -> io::Result<()>
    where
        F: FnOnce(&mut Frame),
    {
        // Resize if needed
        let size = terminal::size()?;
        let area = Rect::new(0, 0, size.0, size.1);
        if self.buffers[self.current].area != area {
            // Clear screen on resize to remove stale content
            self.backend.stdout.queue(CtClear(ClearType::All))?;
            self.buffers[0] = Buffer::empty(area);
            self.buffers[1] = Buffer::empty(area);
        }

        // Clear the current buffer
        let buffer = &mut self.buffers[self.current];
        for cell in &mut buffer.content {
            cell.reset();
        }

        // Run the drawing function
        let mut frame = Frame {
            buffer: &mut self.buffers[self.current],
            cursor_position: None,
        };
        f(&mut frame);

        let cursor_position = frame.cursor_position;

        // Render diff to terminal
        self.flush_diff()?;

        // Handle cursor
        if let Some((x, y)) = cursor_position {
            self.backend.stdout.queue(Show)?;
            self.backend.stdout.queue(MoveTo(x, y))?;
            self.hidden_cursor = false;
        } else if !self.hidden_cursor {
            self.backend.stdout.queue(Hide)?;
            self.hidden_cursor = true;
        }

        self.backend.flush()?;

        // Swap buffers
        self.current = 1 - self.current;

        Ok(())
    }

    fn flush_diff(&mut self) -> io::Result<()> {
        let current = &self.buffers[self.current];
        let previous = &self.buffers[1 - self.current];

        let mut last_fg = Color::Reset;
        let mut last_bg = Color::Reset;
        let mut last_modifier = Modifier::empty();

        for y in current.area.y..current.area.bottom() {
            let mut skip = 0;
            for x in current.area.x..current.area.right() {
                let idx = current.index_of(x, y);
                let cell = &current.content[idx];
                let prev = previous.content.get(idx);

                // Skip continuation cells - they're placeholders for wide characters
                // The wide char already printed and advanced the cursor past this position
                if cell.is_continuation {
                    skip += 1;
                    continue;
                }

                // Skip if same as previous
                if let Some(p) = prev
                    && cell == p {
                        skip += 1;
                        continue;
                    }

                // Position cursor (skip if sequential)
                if skip > 0 || x == current.area.x {
                    self.backend.stdout.queue(MoveTo(x, y))?;
                }
                skip = 0;

                // Set colors/modifiers if changed
                if cell.fg != last_fg {
                    self.backend.stdout.queue(SetForegroundColor(cell.fg.to_crossterm()))?;
                    last_fg = cell.fg;
                }
                if cell.bg != last_bg {
                    self.backend.stdout.queue(SetBackgroundColor(cell.bg.to_crossterm()))?;
                    last_bg = cell.bg;
                }
                if cell.modifier != last_modifier {
                    // Reset first, then apply
                    self.backend.stdout.queue(SetAttribute(Attribute::Reset))?;
                    if cell.modifier.contains(Modifier::BOLD) {
                        self.backend.stdout.queue(SetAttribute(Attribute::Bold))?;
                    }
                    if cell.modifier.contains(Modifier::DIM) {
                        self.backend.stdout.queue(SetAttribute(Attribute::Dim))?;
                    }
                    if cell.modifier.contains(Modifier::ITALIC) {
                        self.backend.stdout.queue(SetAttribute(Attribute::Italic))?;
                    }
                    if cell.modifier.contains(Modifier::UNDERLINED) {
                        self.backend.stdout.queue(SetAttribute(Attribute::Underlined))?;
                    }
                    if cell.modifier.contains(Modifier::REVERSED) {
                        self.backend.stdout.queue(SetAttribute(Attribute::Reverse))?;
                    }
                    // Re-apply colors after reset
                    self.backend.stdout.queue(SetForegroundColor(cell.fg.to_crossterm()))?;
                    self.backend.stdout.queue(SetBackgroundColor(cell.bg.to_crossterm()))?;
                    last_modifier = cell.modifier;
                }

                // Print the character
                self.backend.stdout.queue(Print(&cell.symbol))?;
            }
        }

        Ok(())
    }

    pub fn clear(&mut self) -> io::Result<()> {
        self.backend.stdout.execute(CtClear(ClearType::All))?;
        // Reset both buffers
        let size = terminal::size()?;
        let area = Rect::new(0, 0, size.0, size.1);
        self.buffers[0] = Buffer::empty(area);
        self.buffers[1] = Buffer::empty(area);
        Ok(())
    }

    pub fn show_cursor(&mut self) -> io::Result<()> {
        self.backend.stdout.execute(Show)?;
        self.hidden_cursor = false;
        Ok(())
    }

    pub fn hide_cursor(&mut self) -> io::Result<()> {
        self.backend.stdout.execute(Hide)?;
        self.hidden_cursor = true;
        Ok(())
    }

    pub fn backend_mut(&mut self) -> &mut CrosstermBackend {
        &mut self.backend
    }
}

/// Frame for rendering widgets
pub struct Frame<'a> {
    buffer: &'a mut Buffer,
    cursor_position: Option<(u16, u16)>,
}

impl<'a> Frame<'a> {
    pub fn area(&self) -> Rect {
        self.buffer.area
    }

    pub fn buffer_mut(&mut self) -> &mut Buffer {
        self.buffer
    }

    pub fn render_widget<W: Widget>(&mut self, widget: W, area: Rect) {
        widget.render(area, self.buffer);
    }

    pub fn render_stateful_widget<W: StatefulWidget>(
        &mut self,
        widget: W,
        area: Rect,
        state: &mut W::State,
    ) {
        widget.render(area, self.buffer, state);
    }

    pub fn set_cursor_position(&mut self, position: (u16, u16)) {
        self.cursor_position = Some(position);
    }
}

// ============================================================================
// Widget traits
// ============================================================================

pub trait Widget {
    fn render(self, area: Rect, buf: &mut Buffer);
}

pub trait StatefulWidget {
    type State;
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State);
}

// ============================================================================
// Widgets
// ============================================================================

/// Clear widget - fills area with empty cells
#[derive(Debug, Clone, Copy, Default)]
pub struct Clear;

impl Widget for Clear {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if let Some(cell) = buf.get_mut(x, y) {
                    cell.reset();
                }
            }
        }
    }
}

/// Border types
#[derive(Debug, Clone, Copy, Default)]
pub struct Borders(u8);

impl Borders {
    pub const NONE: Self = Self(0);
    pub const TOP: Self = Self(1);
    pub const BOTTOM: Self = Self(2);
    pub const LEFT: Self = Self(4);
    pub const RIGHT: Self = Self(8);
    pub const ALL: Self = Self(15);

    pub fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl std::ops::BitOr for Borders {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// Block widget - borders and title
#[derive(Debug, Clone, Default)]
pub struct Block<'a> {
    title: Option<Line<'a>>,
    borders: Borders,
    border_style: Style,
    style: Style,
}

impl<'a> Block<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title<T: Into<Line<'a>>>(mut self, title: T) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn borders(mut self, borders: Borders) -> Self {
        self.borders = borders;
        self
    }

    pub fn border_style(mut self, style: Style) -> Self {
        self.border_style = style;
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn inner(&self, area: Rect) -> Rect {
        let mut inner = area;
        if self.borders.contains(Borders::LEFT) {
            inner.x = inner.x.saturating_add(1);
            inner.width = inner.width.saturating_sub(1);
        }
        if self.borders.contains(Borders::TOP) {
            inner.y = inner.y.saturating_add(1);
            inner.height = inner.height.saturating_sub(1);
        }
        if self.borders.contains(Borders::RIGHT) {
            inner.width = inner.width.saturating_sub(1);
        }
        if self.borders.contains(Borders::BOTTOM) {
            inner.height = inner.height.saturating_sub(1);
        }
        inner
    }
}

impl Widget for Block<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }

        // Fill background
        buf.set_style(area, self.style);

        // Draw borders
        let symbols = ("─", "│", "┌", "┐", "└", "┘");

        // Top border
        if self.borders.contains(Borders::TOP) && area.height > 0 {
            for x in area.x + 1..area.right().saturating_sub(1) {
                if let Some(cell) = buf.get_mut(x, area.y) {
                    cell.set_symbol(symbols.0);
                    cell.set_style(self.border_style);
                }
            }
        }

        // Bottom border
        if self.borders.contains(Borders::BOTTOM) && area.height > 1 {
            for x in area.x + 1..area.right().saturating_sub(1) {
                if let Some(cell) = buf.get_mut(x, area.bottom() - 1) {
                    cell.set_symbol(symbols.0);
                    cell.set_style(self.border_style);
                }
            }
        }

        // Left border
        if self.borders.contains(Borders::LEFT) && area.width > 0 {
            for y in area.y + 1..area.bottom().saturating_sub(1) {
                if let Some(cell) = buf.get_mut(area.x, y) {
                    cell.set_symbol(symbols.1);
                    cell.set_style(self.border_style);
                }
            }
        }

        // Right border
        if self.borders.contains(Borders::RIGHT) && area.width > 1 {
            for y in area.y + 1..area.bottom().saturating_sub(1) {
                if let Some(cell) = buf.get_mut(area.right() - 1, y) {
                    cell.set_symbol(symbols.1);
                    cell.set_style(self.border_style);
                }
            }
        }

        // Corners
        if self.borders.contains(Borders::TOP | Borders::LEFT)
            && let Some(cell) = buf.get_mut(area.x, area.y) {
                cell.set_symbol(symbols.2);
                cell.set_style(self.border_style);
            }
        if self.borders.contains(Borders::TOP | Borders::RIGHT) && area.width > 1
            && let Some(cell) = buf.get_mut(area.right() - 1, area.y) {
                cell.set_symbol(symbols.3);
                cell.set_style(self.border_style);
            }
        if self.borders.contains(Borders::BOTTOM | Borders::LEFT) && area.height > 1
            && let Some(cell) = buf.get_mut(area.x, area.bottom() - 1) {
                cell.set_symbol(symbols.4);
                cell.set_style(self.border_style);
            }
        if self.borders.contains(Borders::BOTTOM | Borders::RIGHT) && area.width > 1 && area.height > 1
            && let Some(cell) = buf.get_mut(area.right() - 1, area.bottom() - 1) {
                cell.set_symbol(symbols.5);
                cell.set_style(self.border_style);
            }

        // Title
        if let Some(title) = &self.title {
            let title_x = area.x + 1;
            let title_width = (area.width.saturating_sub(2)) as usize;
            if title_width > 0 {
                buf.set_line(title_x, area.y, title, title_width as u16);
            }
        }
    }
}

/// Text wrapping mode
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Wrap {
    pub trim: bool,
}

impl Wrap {
    pub fn trim(mut self, trim: bool) -> Self {
        self.trim = trim;
        self
    }
}

/// Paragraph widget
#[derive(Debug, Clone, Default)]
pub struct Paragraph<'a> {
    block: Option<Block<'a>>,
    text: Text<'a>,
    style: Style,
    wrap: Option<Wrap>,
}

impl<'a> Paragraph<'a> {
    pub fn new<T: Into<Text<'a>>>(text: T) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }

    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn wrap(mut self, wrap: Wrap) -> Self {
        self.wrap = Some(wrap);
        self
    }
}

impl Widget for Paragraph<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let text_area = if let Some(block) = &self.block {
            let inner = block.inner(area);
            block.clone().render(area, buf);
            inner
        } else {
            area
        };

        if text_area.is_empty() {
            return;
        }

        buf.set_style(text_area, self.style);

        for (i, line) in self.text.lines.iter().enumerate() {
            let y = text_area.y + i as u16;
            if y >= text_area.bottom() {
                break;
            }
            buf.set_line(text_area.x, y, line, text_area.width);
        }
    }
}

impl<'a> From<Line<'a>> for Paragraph<'a> {
    fn from(line: Line<'a>) -> Self {
        Paragraph::new(vec![line])
    }
}

impl<'a> From<Vec<Line<'a>>> for Paragraph<'a> {
    fn from(lines: Vec<Line<'a>>) -> Self {
        Paragraph::new(lines)
    }
}

/// Table cell
#[derive(Debug, Clone, Default)]
pub struct Cell<'a> {
    content: Line<'a>,
    style: Style,
}

impl<'a> Cell<'a> {
    pub fn new<T: Into<Line<'a>>>(content: T) -> Self {
        Self {
            content: content.into(),
            style: Style::default(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl<'a> From<&'a str> for Cell<'a> {
    fn from(s: &'a str) -> Self {
        Cell::new(s)
    }
}

impl<'a> From<String> for Cell<'a> {
    fn from(s: String) -> Self {
        Cell::new(s)
    }
}

impl<'a> From<Line<'a>> for Cell<'a> {
    fn from(line: Line<'a>) -> Self {
        Cell::new(line)
    }
}

impl<'a> From<Span<'a>> for Cell<'a> {
    fn from(span: Span<'a>) -> Self {
        Cell::new(Line::from(span))
    }
}

impl<'a> From<Vec<Span<'a>>> for Cell<'a> {
    fn from(spans: Vec<Span<'a>>) -> Self {
        Cell::new(Line::from(spans))
    }
}

/// Table row
#[derive(Debug, Clone, Default)]
pub struct Row<'a> {
    cells: Vec<Cell<'a>>,
    height: u16,
    style: Style,
}

impl<'a> Row<'a> {
    pub fn new<T: IntoIterator<Item = Cell<'a>>>(cells: T) -> Self {
        Self {
            cells: cells.into_iter().collect(),
            height: 1,
            style: Style::default(),
        }
    }

    pub fn height(mut self, height: u16) -> Self {
        self.height = height;
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

/// Table widget
#[derive(Debug, Clone, Default)]
pub struct Table<'a> {
    block: Option<Block<'a>>,
    header: Option<Row<'a>>,
    rows: Vec<Row<'a>>,
    widths: Vec<Constraint>,
    column_spacing: u16,
    style: Style,
    row_highlight_style: Style,
    highlight_symbol: Option<&'a str>,
}

impl<'a> Table<'a> {
    pub fn new<R: IntoIterator<Item = Row<'a>>, C: Into<Vec<Constraint>>>(rows: R, widths: C) -> Self {
        Self {
            rows: rows.into_iter().collect(),
            widths: widths.into(),
            ..Default::default()
        }
    }

    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }

    pub fn header(mut self, header: Row<'a>) -> Self {
        self.header = Some(header);
        self
    }

    pub fn widths(mut self, widths: impl Into<Vec<Constraint>>) -> Self {
        self.widths = widths.into();
        self
    }

    pub fn column_spacing(mut self, spacing: u16) -> Self {
        self.column_spacing = spacing;
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn row_highlight_style(mut self, style: Style) -> Self {
        self.row_highlight_style = style;
        self
    }

    pub fn highlight_symbol(mut self, symbol: &'a str) -> Self {
        self.highlight_symbol = Some(symbol);
        self
    }

    fn get_column_widths(&self, max_width: u16) -> Vec<u16> {
        if self.widths.is_empty() {
            return vec![];
        }

        let spacing_total = self.column_spacing * (self.widths.len().saturating_sub(1)) as u16;
        let available = max_width.saturating_sub(spacing_total) as i32;

        let mut widths: Vec<i32> = vec![0; self.widths.len()];
        let mut remaining = available;
        let mut flex_count = 0;

        // First pass: fixed sizes (Length, Percentage, Ratio, Max)
        // Min and Fill are flexible - they start at minimum and can grow
        for (i, constraint) in self.widths.iter().enumerate() {
            match constraint {
                Constraint::Length(len) => {
                    widths[i] = (*len as i32).min(remaining);
                    remaining -= widths[i];
                }
                Constraint::Percentage(pct) => {
                    widths[i] = (available * (*pct as i32) / 100).min(remaining);
                    remaining -= widths[i];
                }
                Constraint::Min(min) => {
                    // Reserve minimum, track as flexible
                    widths[i] = (*min as i32).min(remaining);
                    remaining -= widths[i];
                    flex_count += 1;
                }
                Constraint::Max(max) => {
                    widths[i] = (*max as i32).min(remaining);
                    remaining -= widths[i];
                }
                Constraint::Ratio(num, den) => {
                    if *den > 0 {
                        widths[i] = (available * (*num as i32) / (*den as i32)).min(remaining);
                        remaining -= widths[i];
                    }
                }
                Constraint::Fill(_) => {
                    flex_count += 1;
                }
            }
        }

        // Second pass: distribute remaining to flexible columns (Min and Fill)
        if flex_count > 0 && remaining > 0 {
            let per_flex = remaining / flex_count;
            for (i, constraint) in self.widths.iter().enumerate() {
                match constraint {
                    Constraint::Min(_) | Constraint::Fill(_) => {
                        widths[i] += per_flex;
                    }
                    _ => {}
                }
            }
        }

        widths.into_iter().map(|w| w.max(0) as u16).collect()
    }
}

impl Widget for Table<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let table_area = if let Some(block) = &self.block {
            let inner = block.inner(area);
            block.clone().render(area, buf);
            inner
        } else {
            area
        };

        if table_area.is_empty() {
            return;
        }

        // Apply base background style to entire table area
        if let Some(bg) = self.style.bg {
            let bg_style = Style::default().bg(bg);
            buf.set_style(table_area, bg_style);
        }

        let col_widths = self.get_column_widths(table_area.width);
        let mut y = table_area.y;

        // Render header
        if let Some(header) = &self.header
            && y < table_area.bottom() {
                // Apply header row background first
                let header_style = self.style.patch(header.style);
                if let Some(bg) = header_style.bg {
                    buf.set_style(Rect::new(table_area.x, y, table_area.width, 1), Style::default().bg(bg));
                }
                let mut x = table_area.x;
                for (i, cell) in header.cells.iter().enumerate() {
                    if let Some(&width) = col_widths.get(i) {
                        // set_line preserves span styles, don't overwrite them
                        buf.set_line(x, y, &cell.content, width);
                        x += width + self.column_spacing;
                    }
                }
                y += header.height;
            }

        // Render rows
        for row in &self.rows {
            if y >= table_area.bottom() {
                break;
            }
            // Apply row background first
            let row_style = self.style.patch(row.style);
            if let Some(bg) = row_style.bg {
                buf.set_style(Rect::new(table_area.x, y, table_area.width, 1), Style::default().bg(bg));
            }
            let mut x = table_area.x;
            for (i, cell) in row.cells.iter().enumerate() {
                if let Some(&width) = col_widths.get(i) {
                    buf.set_line(x, y, &cell.content, width);
                    x += width + self.column_spacing;
                }
            }
            y += row.height;
        }
    }
}

/// List item
#[derive(Debug, Clone)]
pub struct ListItem<'a> {
    content: Line<'a>,
    style: Style,
}

impl<'a> ListItem<'a> {
    pub fn new<T: Into<Line<'a>>>(content: T) -> Self {
        Self {
            content: content.into(),
            style: Style::default(),
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl<'a> From<&'a str> for ListItem<'a> {
    fn from(s: &'a str) -> Self {
        ListItem::new(s)
    }
}

impl<'a> From<Line<'a>> for ListItem<'a> {
    fn from(line: Line<'a>) -> Self {
        ListItem::new(line)
    }
}

impl<'a> From<Vec<Span<'a>>> for ListItem<'a> {
    fn from(spans: Vec<Span<'a>>) -> Self {
        ListItem::new(Line::from(spans))
    }
}

/// List widget
#[derive(Debug, Clone, Default)]
pub struct List<'a> {
    block: Option<Block<'a>>,
    items: Vec<ListItem<'a>>,
    style: Style,
    highlight_style: Style,
    highlight_symbol: Option<&'a str>,
}

impl<'a> List<'a> {
    pub fn new<T: IntoIterator<Item = ListItem<'a>>>(items: T) -> Self {
        Self {
            items: items.into_iter().collect(),
            ..Default::default()
        }
    }

    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn highlight_style(mut self, style: Style) -> Self {
        self.highlight_style = style;
        self
    }

    pub fn highlight_symbol(mut self, symbol: &'a str) -> Self {
        self.highlight_symbol = Some(symbol);
        self
    }
}

impl Widget for List<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let list_area = if let Some(block) = &self.block {
            let inner = block.inner(area);
            block.clone().render(area, buf);
            inner
        } else {
            area
        };

        if list_area.is_empty() {
            return;
        }

        // Apply base style background to entire list area
        if let Some(bg) = self.style.bg {
            let bg_style = Style::default().bg(bg);
            buf.set_style(list_area, bg_style);
        }

        for (i, item) in self.items.iter().enumerate() {
            let y = list_area.y + i as u16;
            if y >= list_area.bottom() {
                break;
            }
            // Apply item background first (if any), then render line with span styles
            if let Some(bg) = item.style.bg {
                let bg_style = Style::default().bg(bg);
                buf.set_style(Rect::new(list_area.x, y, list_area.width, 1), bg_style);
            }
            buf.set_line(list_area.x, y, &item.content, list_area.width);
        }
    }
}

/// Scrollbar orientation
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScrollbarOrientation {
    #[default]
    VerticalRight,
    VerticalLeft,
    HorizontalBottom,
    HorizontalTop,
}

/// Scrollbar state
#[derive(Debug, Clone, Default)]
pub struct ScrollbarState {
    pub content_length: usize,
    pub position: usize,
    pub viewport_content_length: usize,
}

impl ScrollbarState {
    pub fn new(content_length: usize) -> Self {
        Self {
            content_length,
            position: 0,
            viewport_content_length: 0,
        }
    }

    pub fn content_length(mut self, len: usize) -> Self {
        self.content_length = len;
        self
    }

    pub fn position(mut self, pos: usize) -> Self {
        self.position = pos;
        self
    }

    pub fn viewport_content_length(mut self, len: usize) -> Self {
        self.viewport_content_length = len;
        self
    }
}

/// Scrollbar widget
#[derive(Debug, Clone)]
pub struct Scrollbar<'a> {
    orientation: ScrollbarOrientation,
    thumb_symbol: &'a str,
    track_symbol: Option<&'a str>,
    style: Style,
}

impl<'a> Default for Scrollbar<'a> {
    fn default() -> Self {
        Self {
            orientation: ScrollbarOrientation::VerticalRight,
            thumb_symbol: "█",
            track_symbol: Some("░"),
            style: Style::default(),
        }
    }
}

impl<'a> Scrollbar<'a> {
    pub fn new(orientation: ScrollbarOrientation) -> Self {
        Self {
            orientation,
            ..Default::default()
        }
    }

    pub fn orientation(mut self, orientation: ScrollbarOrientation) -> Self {
        self.orientation = orientation;
        self
    }

    pub fn thumb_symbol(mut self, symbol: &'a str) -> Self {
        self.thumb_symbol = symbol;
        self
    }

    pub fn track_symbol(mut self, symbol: Option<&'a str>) -> Self {
        self.track_symbol = symbol;
        self
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl StatefulWidget for Scrollbar<'_> {
    type State = ScrollbarState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        if area.is_empty() || state.content_length == 0 {
            return;
        }

        let (track_len, _is_vertical) = match self.orientation {
            ScrollbarOrientation::VerticalRight | ScrollbarOrientation::VerticalLeft => {
                (area.height as usize, true)
            }
            ScrollbarOrientation::HorizontalBottom | ScrollbarOrientation::HorizontalTop => {
                (area.width as usize, false)
            }
        };

        if track_len == 0 {
            return;
        }

        // Calculate thumb size and position
        let viewport = state.viewport_content_length.max(1);
        let thumb_size = (track_len * viewport / state.content_length.max(1)).max(1).min(track_len);
        let scrollable = state.content_length.saturating_sub(viewport);
        let thumb_pos = if scrollable > 0 {
            (track_len - thumb_size) * state.position / scrollable
        } else {
            0
        };

        // Draw track and thumb
        for i in 0..track_len {
            let (x, y) = match self.orientation {
                ScrollbarOrientation::VerticalRight => (area.right() - 1, area.y + i as u16),
                ScrollbarOrientation::VerticalLeft => (area.x, area.y + i as u16),
                ScrollbarOrientation::HorizontalBottom => (area.x + i as u16, area.bottom() - 1),
                ScrollbarOrientation::HorizontalTop => (area.x + i as u16, area.y),
            };

            if let Some(cell) = buf.get_mut(x, y) {
                let symbol = if i >= thumb_pos && i < thumb_pos + thumb_size {
                    self.thumb_symbol
                } else {
                    self.track_symbol.unwrap_or(" ")
                };
                cell.set_symbol(symbol);
                cell.set_style(self.style);
            }
        }
    }
}
