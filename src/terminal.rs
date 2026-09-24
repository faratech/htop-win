//! Minimal terminal UI library - replaces ratatui for smaller binary size
//!
//! Provides: Buffer, Terminal, Frame, widgets (Block, Paragraph, Table, List, etc.)

#![allow(dead_code)] // Library provides full API even if not all used

use crossterm::{
    ExecutableCommand, QueueableCommand,
    cursor::{Hide, MoveTo, Show},
    style::{
        Attribute, Color as CtColor, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{self, Clear as CtClear, ClearType},
};
use std::io::{self, BufWriter, Stdout, Write};

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
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Total cell count. Returns `usize` (not `u16`) so terminals with more than
    /// 65535 cells (large/ultrawide/high-DPI, e.g. 300x300) don't saturate the
    /// area, which previously under-allocated `Buffer` and caused an out-of-bounds
    /// index panic in `flush_diff` (process-aborting under `panic = "abort"`).
    pub fn area(&self) -> usize {
        (self.width as usize) * (self.height as usize)
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
        if self.constraints.is_empty() {
            return vec![area];
        }
        if area.is_empty() {
            return vec![Rect::default(); self.constraints.len()];
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
            let mut extra = remaining % flex_count;
            for (i, constraint) in self.constraints.iter().enumerate() {
                match constraint {
                    Constraint::Min(_) | Constraint::Fill(_) => {
                        sizes[i] += per_flex;
                        // Hand out the floor-division remainder one cell at a
                        // time so flex elements together absorb all of it.
                        if extra > 0 {
                            sizes[i] += 1;
                            extra -= 1;
                        }
                    }
                    _ => {}
                }
            }
        } else if remaining > 0 {
            // All constraints fixed (Length/Percentage/Ratio/Max): floor division
            // in the first pass can still leave space. Give it to the last element
            // so the children cover the parent area exactly.
            if let Some(last) = sizes.last_mut() {
                *last += remaining;
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

/// Hashed as one packed word (row keys hash thousands of colors per frame).
impl std::hash::Hash for Color {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u32(self.packed());
    }
}

impl Color {
    /// Injective `u32` encoding: named colors 0..=16, RGB and indexed colors
    /// tagged in the top byte. Never `u32::MAX` (used for "no color").
    fn packed(self) -> u32 {
        match self {
            Color::Reset => 0,
            Color::Black => 1,
            Color::Red => 2,
            Color::Green => 3,
            Color::Yellow => 4,
            Color::Blue => 5,
            Color::Magenta => 6,
            Color::Cyan => 7,
            Color::Gray => 8,
            Color::DarkGray => 9,
            Color::LightRed => 10,
            Color::LightGreen => 11,
            Color::LightYellow => 12,
            Color::LightBlue => 13,
            Color::LightMagenta => 14,
            Color::LightCyan => 15,
            Color::White => 16,
            Color::Rgb(r, g, b) => {
                0x0100_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
            }
            Color::Indexed(i) => 0x0200_0000 | u32::from(i),
        }
    }

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
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
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

/// Hashed as two packed words instead of one write per field and tag.
impl std::hash::Hash for Style {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let color = |c: Option<Color>| u64::from(c.map_or(u32::MAX, Color::packed));
        state.write_u64((color(self.fg) << 32) | color(self.bg));
        state.write_u32(
            (u32::from(self.add_modifier.bits()) << 16) | u32::from(self.sub_modifier.bits()),
        );
    }
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
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

/// Maximum number of UTF-8 bytes a symbol keeps inline inside the cell.
///
/// 15 bytes covers everything the UI draws cell-by-cell: ASCII, box-drawing
/// glyphs (3 B), CJK ideographs (3 B), emoji with variation selectors (7 B)
/// and regional-indicator flag pairs (8 B). Longer grapheme clusters (ZWJ
/// sequences) fall back to the heap.
const SYMBOL_INLINE_CAP: usize = 15;

/// `meta` value marking the heap representation. Everything from 0 to
/// [`SYMBOL_INLINE_CAP`] is the inline byte length instead.
const SYMBOL_HEAP_TAG: u8 = u8::MAX;

/// An empty inline symbol: `head`/`tail` zeroed, `meta` = length 0.
const SYMBOL_INLINE_EMPTY: Symbol = Symbol {
    head: 0,
    tail: [0; 7],
    meta: 0,
};

/// Cell glyph storage: inline byte array for short symbols, heap fallback
/// otherwise. Dereferences to `str`.
///
/// Packed into 16 bytes: `head` (8) + `tail` (7) hold the inline UTF-8 bytes,
/// and `meta` doubles as the discriminant — `0..=SYMBOL_INLINE_CAP` is the
/// inline byte length, [`SYMBOL_HEAP_TAG`] marks the heap form (pointer in
/// `head`, byte length in `tail[0..2]`, capped at 65 535). `repr(C)` makes
/// `head`/`tail` contiguous so the inline text reads as one slice; because
/// the heap pointer is hand-managed, `Clone` and `Drop` are manual.
#[repr(C)]
pub struct Symbol {
    head: usize,
    tail: [u8; 7],
    meta: u8,
}

impl Clone for Symbol {
    fn clone(&self) -> Self {
        // Inline symbols are plain bytes: copy the fields instead of
        // re-validating and re-pushing the text (the per-cell hot path of
        // buffer clears and copies).
        if self.meta != SYMBOL_HEAP_TAG {
            return Symbol {
                head: self.head,
                tail: self.tail,
                meta: self.meta,
            };
        }
        let mut cloned = SYMBOL_INLINE_EMPTY;
        cloned.push_raw(self.as_str());
        cloned
    }
}

impl Symbol {
    /// A one-byte inline symbol (`byte` must be printable ASCII).
    const fn inline_ascii(byte: u8) -> Symbol {
        let mut bytes = [0u8; std::mem::size_of::<usize>()];
        bytes[0] = byte;
        Symbol {
            head: usize::from_ne_bytes(bytes),
            tail: [0; 7],
            meta: 1,
        }
    }

    /// Replace the text with one printable ASCII byte. Writes the same raw
    /// bytes as `clear` + `push_raw`, so inline equality is unaffected.
    #[inline]
    fn set_ascii(&mut self, byte: u8) {
        if self.meta == SYMBOL_HEAP_TAG {
            drop(self.take_heap_box());
        }
        let mut bytes = [0u8; std::mem::size_of::<usize>()];
        bytes[0] = byte;
        self.head = usize::from_ne_bytes(bytes);
        self.tail = [0; 7];
        self.meta = 1;
    }

    fn clear(&mut self) {
        // Back to inline empty; drops any heap allocation immediately so a
        // reused cell does not pin a Box across frames. (ptr::write rather
        // than assignment so the old value is never dropped re-entrantly —
        // see take_heap_box.)
        if self.take_heap_box().is_none() {
            unsafe { std::ptr::write(self, SYMBOL_INLINE_EMPTY) };
        }
    }

    /// Append `text`, mapping terminal-hostile chars exactly like
    /// [`push_terminal_safe_char`] did when symbols were plain `String`s.
    fn push_sanitized_str(&mut self, text: &str) {
        if !text.is_empty() && !text.chars().any(is_terminal_control) {
            self.push_raw(text);
            return;
        }
        for ch in text.chars() {
            self.push_sanitized_char(ch);
        }
    }

    fn push_sanitized_char(&mut self, ch: char) {
        if ch == '\t' {
            self.push_raw(" ");
        } else if is_terminal_control(ch) {
            self.push_raw("\u{FFFD}");
        } else {
            let mut buf = [0u8; 4];
            self.push_raw(ch.encode_utf8(&mut buf));
        }
    }

    /// Append already-sanitized UTF-8 text, spilling to the heap when the
    /// inline array cannot hold it.
    fn push_raw(&mut self, text: &str) {
        if self.meta != SYMBOL_HEAP_TAG && self.len() + text.len() > SYMBOL_INLINE_CAP {
            self.spill_to_heap();
        }
        if self.meta == SYMBOL_HEAP_TAG {
            // Rebuild the box with the appended text (take + realloc keeps the
            // old bytes; push_str extends in place when capacity allows).
            let mut owned = self.take_heap_box().unwrap_or_default().into_string();
            owned.push_str(text);
            self.set_heap(owned.into_boxed_str());
        } else {
            let start = usize::from(self.meta);
            let end = start + text.len();
            debug_assert!(end <= SYMBOL_INLINE_CAP);
            // SAFETY: repr(C) puts the 15 inline bytes at offsets 0..15 and
            // this write stays within the live range [start, end).
            unsafe {
                let base = std::ptr::from_mut(self) as *mut u8;
                std::ptr::copy_nonoverlapping(text.as_ptr(), base.add(start), text.len());
            }
            self.meta = end as u8;
        }
    }

    fn spill_to_heap(&mut self) {
        if self.meta == SYMBOL_HEAP_TAG {
            return;
        }
        let text = self.as_str().to_owned();
        self.set_heap(text.into_boxed_str());
    }

    /// Move the heap representation back into an owned box, resetting `self`
    /// to the empty inline form. `None` when this symbol is inline.
    fn take_heap_box(&mut self) -> Option<Box<str>> {
        if self.meta != SYMBOL_HEAP_TAG {
            return None;
        }
        let len = u16::from(self.tail[0]) | (u16::from(self.tail[1]) << 8);
        let ptr = self.head as *mut u8;
        // Overwrite WITHOUT dropping the old value: Drop::drop routes here,
        // so an assignment (`*self = …`) would drop the heap tag again and
        // re-enter this method on the same pointer — infinite recursion.
        unsafe { std::ptr::write(self, SYMBOL_INLINE_EMPTY) };
        // SAFETY: `ptr`/`len` were produced by `Box::into_raw` in `set_heap`
        // and nothing else freed or copied them since (clear/clone/drop all
        // route through this method or leave the value untouched).
        Some(unsafe {
            Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len as usize) as *mut str)
        })
    }

    fn set_heap(&mut self, boxed: Box<str>) {
        let len = boxed.len().min(u16::MAX as usize);
        let ptr = Box::into_raw(boxed) as *mut u8 as usize;
        self.head = ptr;
        self.tail = [0; 7];
        self.tail[0] = len as u8;
        self.tail[1] = (len >> 8) as u8;
        self.meta = SYMBOL_HEAP_TAG;
    }

    pub fn as_str(&self) -> &str {
        if self.meta == SYMBOL_HEAP_TAG {
            let len = u16::from(self.tail[0]) | (u16::from(self.tail[1]) << 8);
            // SAFETY: heap bytes are valid UTF-8 (only sanitized text is
            // stored) and the pointer stays valid until taken via
            // `take_heap_box`.
            unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                    self.head as *const u8,
                    len as usize,
                ))
            }
        } else {
            // SAFETY: repr(C) lays the inline bytes out contiguously at
            // offset 0; only the first `meta` bytes are live and were stored
            // as valid UTF-8.
            unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                    std::ptr::from_ref(self) as *const u8,
                    usize::from(self.meta),
                ))
            }
        }
    }

    fn is_inline(&self) -> bool {
        self.meta != SYMBOL_HEAP_TAG
    }
}

impl Drop for Symbol {
    fn drop(&mut self) {
        drop(self.take_heap_box());
    }
}

impl Default for Symbol {
    fn default() -> Self {
        let mut symbol = SYMBOL_INLINE_EMPTY;
        symbol.push_raw(" ");
        symbol
    }
}

impl std::ops::Deref for Symbol {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::fmt::Debug for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Show the live text, not the stale tail of the inline array.
        std::fmt::Debug::fmt(self.as_str(), f)
    }
}

impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        if self.meta != other.meta {
            return false;
        }
        if self.meta == SYMBOL_HEAP_TAG {
            self.as_str() == other.as_str()
        } else {
            // Same inline length + same raw head/tail bytes. Appends always
            // write [len..len+n) after a full clear, so equal live text implies
            // equal raw bytes; a false "unequal" from stale tails is possible
            // only in states the cell lifecycle never produces.
            self.head == other.head && self.tail == other.tail
        }
    }
}

impl Eq for Symbol {}

impl PartialEq<str> for Symbol {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Symbol {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// Single cell in the buffer (internal type, not exported as Cell)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferCell {
    pub symbol: Symbol,
    pub fg: Color,
    pub bg: Color,
    pub modifier: Modifier,
    /// True if this cell is a continuation of a wide character in the previous cell
    pub is_continuation: bool,
}

/// A pristine cell (space, default colors), usable in `const` contexts so
/// clears are plain field stores rather than a per-cell symbol rebuild.
pub const DEFAULT_CELL: BufferCell = BufferCell {
    symbol: Symbol::inline_ascii(b' '),
    fg: Color::Reset,
    bg: Color::Reset,
    modifier: Modifier::empty(),
    is_continuation: false,
};

impl Default for BufferCell {
    fn default() -> Self {
        Self {
            symbol: Symbol::default(),
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
        self.symbol.push_sanitized_str(symbol);
        self
    }

    pub fn set_char(&mut self, ch: char) -> &mut Self {
        self.symbol.clear();
        self.symbol.push_sanitized_char(ch);
        self
    }

    /// Write one ASCII byte as a width-1 symbol, sanitized exactly like
    /// `set_char` (tab → space, other controls → U+FFFD).
    #[inline]
    fn set_ascii_byte(&mut self, byte: u8) {
        match byte {
            b'\t' => self.symbol.set_ascii(b' '),
            0x20..=0x7e => self.symbol.set_ascii(byte),
            _ => {
                self.set_char(char::from(byte));
            }
        }
        self.is_continuation = false;
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
        self.symbol.push_sanitized_char(' ');
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

#[inline]
fn push_terminal_safe_char(out: &mut String, ch: char) {
    if ch == '\t' {
        out.push(' ');
    } else if ch.is_control() || ('\u{80}'..='\u{9f}').contains(&ch) {
        out.push('�');
    } else {
        out.push(ch);
    }
}

fn sanitized_terminal_symbol(symbol: &str) -> std::borrow::Cow<'_, str> {
    if symbol
        .chars()
        .all(|ch| ch != '\t' && !ch.is_control() && !('\u{80}'..='\u{9f}').contains(&ch))
    {
        return std::borrow::Cow::Borrowed(symbol);
    }

    let mut safe = String::with_capacity(symbol.len());
    for ch in symbol.chars() {
        push_terminal_safe_char(&mut safe, ch);
    }
    std::borrow::Cow::Owned(safe)
}

#[inline]
fn is_terminal_control(ch: char) -> bool {
    ch == '\t' || ch.is_control() || ('\u{80}'..='\u{9f}').contains(&ch)
}

#[inline]
fn terminal_char_width(ch: char) -> u16 {
    if is_terminal_control(ch) {
        1
    } else {
        unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) as u16
    }
}

#[inline]
fn terminal_symbol_width(symbol: &str) -> u16 {
    if symbol.chars().any(is_terminal_control) {
        1
    } else {
        unicode_width::UnicodeWidthStr::width(symbol) as u16
    }
}

#[derive(Clone, Copy)]
struct TerminalSymbol<'a> {
    text: &'a str,
    width: u16,
}

fn terminal_symbols(text: &str) -> impl Iterator<Item = TerminalSymbol<'_>> {
    use unicode_segmentation::UnicodeSegmentation;
    text.split_inclusive(is_terminal_control)
        .flat_map(|part| part.graphemes(true))
        .map(|text| TerminalSymbol {
            text,
            width: terminal_symbol_width(text),
        })
}

/// Row changed since the compositor last diffed it against the screen.
const ROW_DIRTY: u8 = 1 << 0;
/// Row written during the current frame phase (base layer or overlays); lets
/// the compositor tell its own row paints from other writes.
const ROW_TOUCHED: u8 = 1 << 1;

/// 2D buffer of cells
#[derive(Debug, Clone, Default)]
pub struct Buffer {
    pub area: Rect,
    /// Crate-private so every write goes through `get_mut` (or a bulk helper)
    /// and keeps `row_flags` truthful for the retained compositor.
    pub(crate) content: Vec<BufferCell>,
    /// Per-row `ROW_DIRTY` / `ROW_TOUCHED` bits.
    row_flags: Vec<u8>,
}

impl Buffer {
    pub fn empty(area: Rect) -> Self {
        Self::filled(area, DEFAULT_CELL)
    }

    pub fn filled(area: Rect, cell: BufferCell) -> Self {
        let size = area.area();
        Self {
            area,
            content: vec![cell; size],
            row_flags: vec![ROW_DIRTY; usize::from(area.height)],
        }
    }

    /// Row `row` (0-based from `area.y`) as an index range into `content`.
    fn row_range(&self, row: usize) -> std::ops::Range<usize> {
        let width = usize::from(self.area.width);
        let start = row * width;
        start..(start + width).min(self.content.len())
    }

    /// Overwrite every cell with `template` and mark every row dirty.
    pub(crate) fn fill_all(&mut self, template: &BufferCell) {
        for cell in &mut self.content {
            cell.clone_from(template);
        }
        self.mark_all_dirty();
    }

    /// Overwrite one row with `template` (marks it dirty, not touched: this
    /// is the compositor's own reset, not a widget write).
    pub(crate) fn reset_row(&mut self, row: usize, template: &BufferCell) {
        let range = self.row_range(row);
        for cell in &mut self.content[range] {
            cell.clone_from(template);
        }
        if let Some(flags) = self.row_flags.get_mut(row) {
            *flags |= ROW_DIRTY;
        }
    }

    pub(crate) fn mark_all_dirty(&mut self) {
        for flags in &mut self.row_flags {
            *flags |= ROW_DIRTY;
        }
    }

    fn row_touched(&self, row: usize) -> bool {
        self.row_flags
            .get(row)
            .is_some_and(|flags| flags & ROW_TOUCHED != 0)
    }

    fn clear_touched(&mut self, row: usize) {
        if let Some(flags) = self.row_flags.get_mut(row) {
            *flags &= !ROW_TOUCHED;
        }
    }

    /// Whether row `y` changed since the last diff.
    pub fn row_is_dirty(&self, y: u16) -> bool {
        y.checked_sub(self.area.y)
            .and_then(|row| self.row_flags.get(usize::from(row)))
            .is_some_and(|flags| flags & ROW_DIRTY != 0)
    }

    /// Rows changed since the last diff (what the next flush will visit).
    pub fn dirty_rows(&self) -> usize {
        self.row_flags
            .iter()
            .filter(|flags| **flags & ROW_DIRTY != 0)
            .count()
    }

    /// Resize in place to `area`, reusing the existing cell allocation.
    ///
    /// Equivalent to replacing the buffer with `Buffer::empty(area)` - every
    /// cell ends up pristine - but without freeing and re-allocating the whole
    /// backing store (and, before symbols went inline, every per-cell String)
    /// on each terminal resize. Capacity is kept across shrinks so oscillating
    /// sizes stop thrashing the allocator.
    pub fn resize(&mut self, area: Rect) {
        let size = area.area();
        self.area = area;
        self.content.truncate(size);
        self.content.resize(size, DEFAULT_CELL);
        self.content.fill(DEFAULT_CELL);
        self.row_flags.clear();
        self.row_flags.resize(usize::from(area.height), ROW_DIRTY);
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
            if let Some(flags) = self.row_flags.get_mut(usize::from(y - self.area.y)) {
                *flags |= ROW_DIRTY | ROW_TOUCHED;
            }
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

    pub fn set_string_truncated(
        &mut self,
        x: u16,
        y: u16,
        string: &str,
        max_width: u16,
        style: Style,
    ) {
        let mut col = x;
        let max_col = x
            .saturating_add(max_width)
            .min(self.area.x + self.area.width);

        // ASCII fast path: every byte is one width-1 cell, so skip grapheme
        // segmentation and width lookups entirely.
        if string.is_ascii() {
            for &byte in string.as_bytes() {
                if col >= max_col {
                    break;
                }
                if let Some(cell) = self.get_mut(col, y) {
                    cell.set_ascii_byte(byte);
                    cell.set_style(style);
                }
                col += 1;
            }
            return;
        }

        let mut last_base_col = None;
        for symbol in terminal_symbols(string) {
            let width = symbol.width;
            if width == 0 {
                if let Some(base_col) = last_base_col
                    && let Some(cell) = self.get_mut(base_col, y)
                {
                    cell.symbol.push_sanitized_str(symbol.text);
                }
                continue;
            }
            if col.saturating_add(width) > max_col {
                break;
            }
            self.set_symbol_at(col, y, symbol.text, width, style);
            last_base_col = Some(col);
            col = col.saturating_add(width);
        }
    }

    pub fn set_line(&mut self, x: u16, y: u16, line: &Line<'_>, max_width: u16) {
        self.set_spans(x, y, &line.spans, line.style, max_width);
    }

    /// `set_line` for a line with these spans and style.
    pub(crate) fn set_spans(
        &mut self,
        x: u16,
        y: u16,
        spans: &[Span<'_>],
        line_style: Style,
        max_width: u16,
    ) {
        let Some(row) = self.row_of(y) else {
            return;
        };
        let max_col = x
            .saturating_add(max_width)
            .min(self.area.x + self.area.width);
        let left = self.area.x;
        let range = self.row_range(row);
        let touched = RowCells {
            cells: &mut self.content[range],
            left,
            touched: false,
        }
        .set_line(x, max_col, spans, line_style);
        if touched {
            self.row_flags[row] |= ROW_DIRTY | ROW_TOUCHED;
        }
    }

    fn set_symbol_at(&mut self, col: u16, y: u16, symbol: &str, width: u16, style: Style) {
        let Some(row) = self.row_of(y) else {
            return;
        };
        let left = self.area.x;
        let range = self.row_range(row);
        let mut cells = RowCells {
            cells: &mut self.content[range],
            left,
            touched: false,
        };
        cells.set_symbol_at(col, symbol, width, style);
        if cells.touched {
            self.row_flags[row] |= ROW_DIRTY | ROW_TOUCHED;
        }
    }

    /// Row index of screen line `y`, if inside the buffer.
    fn row_of(&self, y: u16) -> Option<usize> {
        (y >= self.area.y && y < self.area.y + self.area.height)
            .then(|| usize::from(y - self.area.y))
    }

    /// Paint row `row` from scratch: the same cells as `reset_row` to the
    /// background, then `set_style` of `spec.base` over its span and, per
    /// segment, `set_style` and `set_line`, in fewer passes (template fills,
    /// then text written straight into the row).
    pub(crate) fn paint_fresh_row(&mut self, row: usize, background: Color, spec: &RowSpec<'_>) {
        let blank = blank_cell(background);
        let mut base = blank.clone();
        base.set_style(spec.base);
        let left = self.area.x;
        let range = self.row_range(row);
        let cells = &mut self.content[range];
        let len = cells.len();
        let start = usize::from(spec.x.saturating_sub(left)).min(len);
        let end =
            usize::from(spec.x.saturating_add(spec.width).saturating_sub(left)).clamp(start, len);
        for cell in &mut cells[..start] {
            cell.clone_from(&blank);
        }
        for cell in &mut cells[start..end] {
            cell.clone_from(&base);
        }
        for cell in &mut cells[end..] {
            cell.clone_from(&blank);
        }
        let mut row_cells = RowCells {
            cells,
            left,
            touched: true,
        };
        for seg in spec.segs {
            if seg.style != Style::default() {
                row_cells.patch_style(seg.x, seg.width, spec.base.patch(seg.style));
            }
            let max_col = seg
                .x
                .saturating_add(seg.width)
                .min(self.area.x + self.area.width);
            row_cells.set_line(seg.x, max_col, seg.spans, seg.line_style);
        }
        self.row_flags[row] |= ROW_DIRTY | ROW_TOUCHED;
    }

    pub fn set_span(&mut self, x: u16, y: u16, span: &Span<'_>, max_width: u16) {
        let line = Line::from(span.clone());
        self.set_line(x, y, &line, max_width);
    }

    /// Paint only the background color of every cell in `area`. Equivalent to
    /// `set_style(area, Style::default().bg(color))` without per-cell branchwork.
    pub fn fill_bg(&mut self, area: Rect, color: Color) {
        for y in area.y..area.y.saturating_add(area.height) {
            for x in area.x..area.x.saturating_add(area.width) {
                if let Some(cell) = self.get_mut(x, y) {
                    cell.bg = color;
                }
            }
        }
    }

    pub fn set_style(&mut self, area: Rect, style: Style) {
        // A fully default style patches nothing — skip the whole-area sweep
        // (default Block/Table backgrounds hit this every frame).
        if style.fg.is_none()
            && style.bg.is_none()
            && style.add_modifier.is_empty()
            && style.sub_modifier.is_empty()
        {
            return;
        }
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
    stdout: BufWriter<Stdout>,
}

impl CrosstermBackend {
    pub fn new(stdout: Stdout) -> Self {
        // Buffer ANSI output across the frame: without this, raw Stdout's
        // ~1 KB LineWriter turns every queued command into its own write
        // syscall on full repaints. Terminal::draw's single flush() is the
        // only write boundary; crossterm itself flushes before any non-ANSI
        // WinAPI-fallback command, so legacy consoles keep correct ordering.
        Self {
            stdout: BufWriter::with_capacity(256 * 1024, stdout),
        }
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

// ============================================================================
// Retained composition
// ============================================================================

/// One styled run within a screen row: a table cell, a header meter, or a
/// whole line. Painted like `Table`/`Paragraph` paint cells.
pub struct RowSeg<'a> {
    pub x: u16,
    pub width: u16,
    /// Applied over the row's base style before the spans (skipped when default).
    pub style: Style,
    /// The line's own style, under each span's (`Line::style`).
    pub line_style: Style,
    /// The line's spans: any slice, so callers can paint straight from a
    /// shared span buffer instead of building a `Line` per segment.
    pub spans: &'a [Span<'a>],
}

impl<'a> RowSeg<'a> {
    /// A segment painting `line`.
    pub fn line(x: u16, width: u16, style: Style, line: &'a Line<'a>) -> Self {
        Self {
            x,
            width,
            style,
            line_style: line.style,
            spans: &line.spans,
        }
    }
}

/// Everything that paints one screen row. `Frame::paint_row` hashes it as the
/// row's memo key, so the key covers exactly what the painter reads.
pub struct RowSpec<'a> {
    pub x: u16,
    pub width: u16,
    /// Applied across `x..x + width` first (a table row style, a paragraph style).
    pub base: Style,
    pub segs: &'a [RowSeg<'a>],
}

// Row keys cover every field; geometry is packed into one word per run.
impl std::hash::Hash for RowSeg<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u32((u32::from(self.x) << 16) | u32::from(self.width));
        self.style.hash(state);
        self.line_style.hash(state);
        self.spans.hash(state);
    }
}

impl std::hash::Hash for RowSpec<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u32((u32::from(self.x) << 16) | u32::from(self.width));
        self.base.hash(state);
        self.segs.hash(state);
    }
}

/// Key domains, so a painted row can never share a key with a blank one.
const PAINTED_ROW: u8 = 1;
const BLANK_ROW: u8 = 2;

/// Content key per row of the retained back buffer.
pub(crate) struct RowMemo {
    keys: Vec<Option<u64>>,
    /// Keyed (per-process random) SipHash: a crafted process name cannot
    /// force a collision that would leave a changed row unpainted.
    hasher: std::hash::RandomState,
}

impl RowMemo {
    fn new(height: u16) -> Self {
        Self {
            keys: vec![None; usize::from(height)],
            hasher: std::hash::RandomState::new(),
        }
    }

    fn reset(&mut self, height: u16) {
        self.keys.clear();
        self.keys.resize(usize::from(height), None);
    }

    fn invalidate_all(&mut self) {
        self.keys.fill(None);
    }

    fn key<T: std::hash::Hash>(&self, value: &T) -> u64 {
        use std::hash::BuildHasher;
        self.hasher.hash_one(value)
    }
}

/// A blank base-layer cell on `background`.
/// One buffer row as a slice, addressed by screen column: the per-cell writes
/// of `Buffer::set_line` without a bounds check and flag update per cell.
struct RowCells<'a> {
    cells: &'a mut [BufferCell],
    /// Screen column of `cells[0]`.
    left: u16,
    /// Some cell was written (the caller marks the row).
    touched: bool,
}

impl RowCells<'_> {
    fn cell(&mut self, col: u16) -> Option<&mut BufferCell> {
        let cell = self
            .cells
            .get_mut(usize::from(col.checked_sub(self.left)?))?;
        self.touched = true;
        Some(cell)
    }

    /// `Buffer::set_style` over columns `x..x + width` of this row.
    fn patch_style(&mut self, x: u16, width: u16, style: Style) {
        if style.fg.is_none()
            && style.bg.is_none()
            && style.add_modifier.is_empty()
            && style.sub_modifier.is_empty()
        {
            return;
        }
        for col in x..x.saturating_add(width) {
            if let Some(cell) = self.cell(col) {
                cell.set_style(style);
            }
        }
    }

    /// `Buffer::set_line` of a line with these spans and style from column
    /// `x`, stopping before `max_col`. Returns whether any cell was written.
    fn set_line(&mut self, x: u16, max_col: u16, spans: &[Span<'_>], line_style: Style) -> bool {
        let mut col = x;
        let mut last_base_col = None;
        for span in spans {
            let style = line_style.patch(span.style);
            // ASCII fast path (see `set_string_truncated`). Zero-width marks
            // in a later non-ASCII span still attach to the last cell here.
            if span.content.is_ascii() {
                for &byte in span.content.as_bytes() {
                    if col >= max_col {
                        return self.touched;
                    }
                    if let Some(cell) = self.cell(col) {
                        cell.set_ascii_byte(byte);
                        cell.set_style(style);
                    }
                    last_base_col = Some(col);
                    col += 1;
                }
                continue;
            }
            for symbol in terminal_symbols(&span.content) {
                let width = symbol.width;
                if width == 0 {
                    if let Some(base_col) = last_base_col
                        && let Some(cell) = self.cell(base_col)
                    {
                        cell.symbol.push_sanitized_str(symbol.text);
                    }
                    continue;
                }
                if col.saturating_add(width) > max_col {
                    return self.touched;
                }
                self.set_symbol_at(col, symbol.text, width, style);
                last_base_col = Some(col);
                col = col.saturating_add(width);
            }
        }
        self.touched
    }

    fn set_symbol_at(&mut self, col: u16, symbol: &str, width: u16, style: Style) {
        if let Some(cell) = self.cell(col) {
            cell.set_symbol(symbol);
            cell.set_style(style);
            cell.is_continuation = false;
        }
        // These cells are occupied by the wide symbol but contain no content.
        for i in 1..width {
            if let Some(cont_cell) = self.cell(col + i) {
                cont_cell.set_continuation();
                cont_cell.set_style(style);
            }
        }
    }
}

fn blank_cell(background: Color) -> BufferCell {
    let mut cell = DEFAULT_CELL;
    cell.bg = background;
    cell
}

/// Retained frame composer. The back buffer persists across frames, and only
/// rows whose content key changed are reset and repainted (see
/// [`Frame::paint_row`]); unchanged rows are not touched at all, which is
/// what makes a frame cheap (its cost follows the memory it touches). Needs
/// no console, so tests drive it directly.
pub struct Compositor {
    back: Buffer,
    memo: RowMemo,
}

impl Compositor {
    pub fn new(area: Rect) -> Self {
        Self {
            back: Buffer::empty(area),
            memo: RowMemo::new(area.height),
        }
    }

    pub fn area(&self) -> Rect {
        self.back.area
    }

    /// The composed frame.
    pub fn buffer(&self) -> &Buffer {
        &self.back
    }

    /// Resize to `area`: everything repaints on the next frame.
    pub fn resize(&mut self, area: Rect) {
        self.back.resize(area);
        self.memo.reset(area.height);
    }

    /// Forget all row keys and mark every row dirty (e.g. after the screen
    /// was cleared underneath us).
    pub fn invalidate(&mut self) {
        self.memo.invalidate_all();
        self.back.mark_all_dirty();
    }

    /// Compose one frame into the back buffer and return the requested cursor
    /// position. Rows left dirty are what the next diff visits.
    pub fn draw<F>(&mut self, area: Rect, f: F) -> Option<(u16, u16)>
    where
        F: FnOnce(&mut Frame),
    {
        if self.back.area != area {
            self.resize(area);
        }
        let mut frame = Frame::retained(&mut self.back, &mut self.memo);
        f(&mut frame);
        frame.finish()
    }

    /// Treat the composed frame as delivered: clear every dirty flag without
    /// emitting anything (tests measuring what a frame repainted).
    pub fn acknowledge(&mut self) {
        for flags in &mut self.back.row_flags {
            *flags &= !ROW_DIRTY;
        }
    }
}

/// Receives the terminal commands a frame diff produces. The console writer
/// implements it with crossterm; tests implement it with a screen emulator,
/// so both run the exact same diff.
pub(crate) trait DiffSink {
    fn move_to(&mut self, x: u16, y: u16) -> io::Result<()>;
    fn set_fg(&mut self, color: Color) -> io::Result<()>;
    fn set_bg(&mut self, color: Color) -> io::Result<()>;
    fn set_attribute(&mut self, attribute: Attribute) -> io::Result<()>;
    fn print(&mut self, symbol: &str) -> io::Result<()>;
}

impl DiffSink for BufWriter<Stdout> {
    fn move_to(&mut self, x: u16, y: u16) -> io::Result<()> {
        self.queue(MoveTo(x, y)).map(|_| ())
    }

    fn set_fg(&mut self, color: Color) -> io::Result<()> {
        self.queue(SetForegroundColor(color.to_crossterm()))
            .map(|_| ())
    }

    fn set_bg(&mut self, color: Color) -> io::Result<()> {
        self.queue(SetBackgroundColor(color.to_crossterm()))
            .map(|_| ())
    }

    fn set_attribute(&mut self, attribute: Attribute) -> io::Result<()> {
        self.queue(SetAttribute(attribute)).map(|_| ())
    }

    fn print(&mut self, symbol: &str) -> io::Result<()> {
        self.queue(Print(symbol)).map(|_| ())
    }
}

/// Display width of a cell's symbol (at least 1).
fn cell_width(cell: &BufferCell) -> usize {
    let symbol = cell.symbol.as_str();
    if symbol.len() == 1 {
        1
    } else {
        usize::from(terminal_symbol_width(symbol)).max(1)
    }
}

/// Make a row renderable before diffing it: a wide lead whose continuation
/// cells are missing becomes a blank, and a continuation cell no lead covers
/// becomes a blank. Overlays (dialog borders) can land on half of a wide
/// glyph; the terminal erases the whole glyph then, so without this `front`
/// and the screen disagree and the remnant is never repaired.
fn normalize_wide_row(buffer: &mut Buffer, row: usize) {
    let range = buffer.row_range(row);
    let cells = &mut buffer.content[range];
    let mut covered_until = 0;
    for x in 0..cells.len() {
        if cells[x].is_continuation {
            if x >= covered_until {
                cells[x].symbol.set_ascii(b' ');
                cells[x].is_continuation = false;
            }
            continue;
        }
        let width = cell_width(&cells[x]);
        if width > 1 {
            let end = x + width;
            if end <= cells.len() && cells[x + 1..end].iter().all(|c| c.is_continuation) {
                covered_until = end;
            } else {
                cells[x].symbol.set_ascii(b' ');
            }
        }
    }
}

/// Diff the dirty rows of `back` against `front` (the mirror of the screen),
/// emit commands for the cells that changed, and update `front` to match.
/// Clean rows are skipped without being read. Returns false (emitting
/// nothing) when no row was dirty.
pub(crate) fn diff_frame(
    back: &mut Buffer,
    front: &mut Buffer,
    sink: &mut impl DiffSink,
) -> io::Result<bool> {
    debug_assert_eq!(back.area, front.area);
    if !back.row_flags.iter().any(|flags| flags & ROW_DIRTY != 0) {
        return Ok(false);
    }

    sink.set_attribute(Attribute::Reset)?;
    sink.set_fg(Color::Reset)?;
    sink.set_bg(Color::Reset)?;
    let mut last_fg = Color::Reset;
    let mut last_bg = Color::Reset;
    let mut last_modifier = Modifier::empty();

    for row in 0..back.row_flags.len() {
        if back.row_flags[row] & ROW_DIRTY == 0 {
            continue;
        }
        back.row_flags[row] &= !ROW_DIRTY;
        normalize_wide_row(back, row);

        let range = back.row_range(row);
        // Dirty is conservative (a repaint may reproduce the same cells).
        if front.content.get(range.clone()) == back.content.get(range.clone()) {
            continue;
        }
        let y = back.area.y + row as u16;
        let base = range.start;
        let width = range.len();
        let mut need_move = true;
        for x in 0..width {
            let i = base + x;
            let cell = &back.content[i];
            if cell.is_continuation {
                // Printed with its lead; the cursor is no longer sequential.
                if front.content[i] != *cell {
                    front.content[i].clone_from(cell);
                }
                need_move = true;
                continue;
            }
            // A lead that is unchanged still has to be printed again when one
            // of its continuation cells differs on screen.
            let changed = front.content[i] != *cell
                || (x + 1..width)
                    .take_while(|&j| back.content[base + j].is_continuation)
                    .any(|j| front.content[base + j] != back.content[base + j]);
            if !changed {
                need_move = true;
                continue;
            }

            if need_move {
                sink.move_to(back.area.x + x as u16, y)?;
                need_move = false;
            }
            if cell.fg != last_fg {
                sink.set_fg(cell.fg)?;
                last_fg = cell.fg;
            }
            if cell.bg != last_bg {
                sink.set_bg(cell.bg)?;
                last_bg = cell.bg;
            }
            if cell.modifier != last_modifier {
                // Only reset if we're removing attributes; add new ones directly
                let removed = last_modifier.difference(cell.modifier);
                if !removed.is_empty() {
                    // Must reset to remove attributes, then re-apply what's needed
                    sink.set_attribute(Attribute::Reset)?;
                    sink.set_fg(cell.fg)?;
                    sink.set_bg(cell.bg)?;
                    last_fg = cell.fg;
                    last_bg = cell.bg;
                }
                // Apply active modifiers (only new ones if no reset, all if reset occurred)
                let to_apply = if removed.is_empty() {
                    cell.modifier.difference(last_modifier)
                } else {
                    cell.modifier
                };
                for (flag, attribute) in [
                    (Modifier::BOLD, Attribute::Bold),
                    (Modifier::DIM, Attribute::Dim),
                    (Modifier::ITALIC, Attribute::Italic),
                    (Modifier::UNDERLINED, Attribute::Underlined),
                    (Modifier::REVERSED, Attribute::Reverse),
                ] {
                    if to_apply.contains(flag) {
                        sink.set_attribute(attribute)?;
                    }
                }
                last_modifier = cell.modifier;
            }

            // Print the character. Defense-in-depth: cells are sanitized at
            // write time, but sanitize again here so direct buffer mutation
            // can never emit process-controlled ESC/C0/C1 controls.
            let symbol = sanitized_terminal_symbol(&cell.symbol);
            sink.print(symbol.as_ref())?;
            front.content[i].clone_from(cell);
        }
    }

    sink.set_attribute(Attribute::Reset)?;
    sink.set_fg(Color::Reset)?;
    sink.set_bg(Color::Reset)?;
    Ok(true)
}

/// Timing and volume of the last `Terminal::draw` (benchmark reporting).
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameStats {
    /// Running the UI into the retained back buffer.
    pub compose: std::time::Duration,
    /// Diffing, encoding and writing to the console.
    pub output: std::time::Duration,
    /// Rows the diff visited (repainted or otherwise changed).
    pub dirty_rows: u16,
    /// Bytes written to the console.
    pub bytes: usize,
}

/// Reads the console window size. On Windows it keeps one handle to the
/// active screen buffer: crossterm's `terminal::size()` opens and closes
/// `CONOUT$` around every query, three console round trips (~80 µs).
struct ConsoleSize {
    #[cfg(windows)]
    handle: Option<windows::Win32::Foundation::HANDLE>,
}

impl ConsoleSize {
    /// Open the handle to the screen buffer active now (so call after
    /// entering the alternate screen).
    fn open() -> Self {
        #[cfg(windows)]
        {
            use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
            use windows::Win32::Storage::FileSystem::{
                CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
                OPEN_EXISTING,
            };
            let handle = unsafe {
                CreateFileW(
                    windows::core::w!("CONOUT$"),
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAGS_AND_ATTRIBUTES(0),
                    None,
                )
            };
            Self {
                handle: handle.ok(),
            }
        }
        #[cfg(not(windows))]
        Self {}
    }

    /// Window size in cells, like `terminal::size()` (its fallback).
    fn read(&self) -> io::Result<(u16, u16)> {
        #[cfg(windows)]
        if let Some(handle) = self.handle {
            use windows::Win32::System::Console::{
                CONSOLE_SCREEN_BUFFER_INFO, GetConsoleScreenBufferInfo,
            };
            let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
            if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) }.is_ok() {
                let window = info.srWindow;
                return Ok((
                    (window.Right - window.Left + 1) as u16,
                    (window.Bottom - window.Top + 1) as u16,
                ));
            }
        }
        terminal::size()
    }
}

#[cfg(windows)]
impl Drop for ConsoleSize {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
            }
        }
    }
}

/// Terminal wrapper
pub struct Terminal {
    backend: CrosstermBackend,
    compositor: Compositor,
    /// What the screen shows; the diff keeps it in step.
    front: Buffer,
    hidden_cursor: bool,
    last_frame: FrameStats,
    console: ConsoleSize,
    /// Console size as last read (see `refresh_size`).
    size: Rect,
}

impl Terminal {
    pub fn new(backend: CrosstermBackend) -> io::Result<Self> {
        let console = ConsoleSize::open();
        let (width, height) = console.read()?;
        let area = Rect::new(0, 0, width, height);
        Ok(Self {
            backend,
            compositor: Compositor::new(area),
            front: Buffer::empty(area),
            hidden_cursor: false,
            last_frame: FrameStats::default(),
            console,
            size: area,
        })
    }

    /// Re-read the console size; true when it changed (the caller redraws).
    /// The event loop calls this after a frame is out and on resize events,
    /// rather than `draw` querying the console before every frame.
    pub fn refresh_size(&mut self) -> io::Result<bool> {
        let (width, height) = self.console.read()?;
        let size = Rect::new(0, 0, width, height);
        let changed = size != self.size;
        self.size = size;
        Ok(changed)
    }

    /// Draw a frame at the size last read by `refresh_size`.
    pub fn draw<F>(&mut self, f: F) -> io::Result<()>
    where
        F: FnOnce(&mut Frame),
    {
        let area = self.size;
        if self.compositor.area() != area {
            // Clear screen on resize to remove stale content; the screen (and
            // so `front`) is blank afterwards and every row repaints.
            self.backend.stdout.queue(CtClear(ClearType::All))?;
            self.front.resize(area);
            self.compositor.resize(area);
        }

        let compose_start = std::time::Instant::now();
        let cursor_position = self.compositor.draw(area, f);
        let output_start = std::time::Instant::now();
        let dirty_rows = self.compositor.back.dirty_rows();

        diff_frame(
            &mut self.compositor.back,
            &mut self.front,
            &mut self.backend.stdout,
        )?;

        // Handle cursor
        if let Some((x, y)) = cursor_position {
            self.backend.stdout.queue(Show)?;
            self.backend.stdout.queue(MoveTo(x, y))?;
            self.hidden_cursor = false;
        } else if !self.hidden_cursor {
            self.backend.stdout.queue(Hide)?;
            self.hidden_cursor = true;
        }

        let bytes = self.backend.stdout.buffer().len();
        self.backend.flush()?;
        self.last_frame = FrameStats {
            compose: output_start - compose_start,
            output: output_start.elapsed(),
            dirty_rows: dirty_rows.min(usize::from(u16::MAX)) as u16,
            bytes,
        };
        Ok(())
    }

    /// Timing and volume of the last `draw`.
    pub fn last_frame(&self) -> FrameStats {
        self.last_frame
    }

    /// Clear the screen and repaint everything on the next `draw` (Ctrl+L).
    pub fn clear(&mut self) -> io::Result<()> {
        self.backend.stdout.execute(CtClear(ClearType::All))?;
        self.refresh_size()?;
        let area = self.size;
        // A cleared screen holds default cells, so that is what `front` says;
        // retained rows must be sent again even though their keys still match.
        self.front.resize(area);
        if self.compositor.area() == area {
            self.compositor.invalidate();
        } else {
            self.compositor.resize(area);
        }
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

/// Frame for rendering widgets.
///
/// The base layer (header, tabs, table, footer) is painted row by row through
/// [`Frame::paint_row`] between [`Frame::begin`] and [`Frame::finish_base`];
/// overlays (dialogs, error banner) are ordinary widgets drawn afterwards. In
/// a retained frame (from [`Compositor`]) unchanged rows are skipped; a
/// standalone frame (`Frame::new`) always paints everything.
pub struct Frame<'a> {
    buffer: &'a mut Buffer,
    memo: Option<&'a mut RowMemo>,
    cursor_position: Option<(u16, u16)>,
    /// Base-layer background, set by `begin`.
    background: Option<Color>,
    /// Rows claimed by `paint_row` this frame.
    claimed: Vec<bool>,
    base_finished: bool,
}

impl<'a> Frame<'a> {
    pub fn new(buffer: &'a mut Buffer) -> Self {
        let rows = usize::from(buffer.area.height);
        Self {
            buffer,
            memo: None,
            cursor_position: None,
            background: None,
            claimed: vec![false; rows],
            base_finished: false,
        }
    }

    fn retained(buffer: &'a mut Buffer, memo: &'a mut RowMemo) -> Self {
        let mut frame = Self::new(buffer);
        frame.memo = Some(memo);
        frame
    }

    pub fn area(&self) -> Rect {
        self.buffer.area
    }

    pub fn buffer_mut(&mut self) -> &mut Buffer {
        self.buffer
    }

    /// Start the base layer on `background`. A standalone frame clears the
    /// whole buffer to it; a retained frame leaves unchanged rows alone.
    pub fn begin(&mut self, background: Color) {
        self.background = Some(background);
        if self.memo.is_none() {
            self.buffer.fill_all(&blank_cell(background));
        }
    }

    /// Paint one base-layer row: reset it to the background, apply
    /// `spec.base` across `spec.x..spec.x + spec.width`, then each segment's
    /// style and line (exactly how `Table` and `Paragraph` paint). In a
    /// retained frame the row is left untouched when its key (a hash of
    /// `spec` and the background) matches what it already shows.
    pub fn paint_row(&mut self, y: u16, spec: &RowSpec<'_>) {
        let area = self.buffer.area;
        if y < area.y || y >= area.bottom() {
            return;
        }
        let row = usize::from(y - area.y);
        let background = self.background.unwrap_or(Color::Reset);
        let repeat = std::mem::replace(&mut self.claimed[row], true);
        debug_assert!(!repeat, "row {y} painted twice in one frame");

        if let Some(memo) = self.memo.as_deref_mut() {
            let key = memo.key(&(PAINTED_ROW, background, spec));
            if !repeat && !self.buffer.row_touched(row) && memo.keys[row] == Some(key) {
                return;
            }
            // A second paint layers onto the first; its key is not the row's.
            memo.keys[row] = (!repeat).then_some(key);
        }

        if repeat {
            self.buffer
                .set_style(Rect::new(spec.x, y, spec.width, 1), spec.base);
            for seg in spec.segs {
                if seg.style != Style::default() {
                    self.buffer.set_style(
                        Rect::new(seg.x, y, seg.width, 1),
                        spec.base.patch(seg.style),
                    );
                }
                self.buffer
                    .set_spans(seg.x, y, seg.spans, seg.line_style, seg.width);
            }
        } else {
            self.buffer.paint_fresh_row(row, background, spec);
        }
        if self.memo.is_some() && !repeat {
            self.buffer.clear_touched(row);
        }
    }

    /// A single styled line in `area` (a one-line `Paragraph`).
    pub fn paint_line(&mut self, area: Rect, line: &Line<'_>, style: Style) {
        if area.is_empty() {
            return;
        }
        let segs = [RowSeg::line(area.x, area.width, Style::default(), line)];
        self.paint_row(
            area.y,
            &RowSpec {
                x: area.x,
                width: area.width,
                base: style,
                segs: &segs,
            },
        );
    }

    /// End the base layer: rows no `paint_row` claimed become blank (reset
    /// only if they weren't already). Everything drawn afterwards is an
    /// overlay, whose rows repaint on the next frame.
    pub fn finish_base(&mut self) {
        if std::mem::replace(&mut self.base_finished, true) {
            return;
        }
        let Some(memo) = self.memo.as_deref_mut() else {
            return;
        };
        let background = self.background.unwrap_or(Color::Reset);
        let blank_key = memo.key(&(BLANK_ROW, background));
        let blank = blank_cell(background);
        let mut stray = false;
        for (row, claimed) in self.claimed.iter().enumerate() {
            let touched = self.buffer.row_touched(row);
            // A base-layer write outside paint_row bypassed the row keys.
            stray |= touched;
            if *claimed {
                if touched {
                    memo.keys[row] = None;
                }
            } else if touched || memo.keys[row] != Some(blank_key) {
                self.buffer.reset_row(row, &blank);
                memo.keys[row] = Some(blank_key);
            }
            self.buffer.clear_touched(row);
        }
        debug_assert!(!stray, "base-layer write outside Frame::paint_row");
        if stray {
            // Release builds: never let the bypass go stale for long.
            memo.invalidate_all();
        }
    }

    /// Close the frame: rows overlays drew on repaint next frame. Returns the
    /// requested cursor position.
    fn finish(mut self) -> Option<(u16, u16)> {
        self.finish_base();
        if let Some(memo) = self.memo.as_deref_mut() {
            for row in 0..self.claimed.len() {
                if self.buffer.row_touched(row) {
                    memo.keys[row] = None;
                    self.buffer.clear_touched(row);
                }
            }
        }
        self.cursor_position
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
            && let Some(cell) = buf.get_mut(area.x, area.y)
        {
            cell.set_symbol(symbols.2);
            cell.set_style(self.border_style);
        }
        if self.borders.contains(Borders::TOP | Borders::RIGHT)
            && area.width > 1
            && let Some(cell) = buf.get_mut(area.right() - 1, area.y)
        {
            cell.set_symbol(symbols.3);
            cell.set_style(self.border_style);
        }
        if self.borders.contains(Borders::BOTTOM | Borders::LEFT)
            && area.height > 1
            && let Some(cell) = buf.get_mut(area.x, area.bottom() - 1)
        {
            cell.set_symbol(symbols.4);
            cell.set_style(self.border_style);
        }
        if self.borders.contains(Borders::BOTTOM | Borders::RIGHT)
            && area.width > 1
            && area.height > 1
            && let Some(cell) = buf.get_mut(area.right() - 1, area.bottom() - 1)
        {
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

        let mut y = text_area.y;
        for line in &self.text.lines {
            if y >= text_area.bottom() {
                break;
            }
            if let Some(wrap) = self.wrap {
                for wrapped in wrap_line(line, text_area.width as usize, wrap.trim) {
                    if y >= text_area.bottom() {
                        break;
                    }
                    buf.set_line(text_area.x, y, &wrapped, text_area.width);
                    y = y.saturating_add(1);
                }
            } else {
                buf.set_line(text_area.x, y, line, text_area.width);
                y = y.saturating_add(1);
            }
        }
    }
}

fn wrap_line(line: &Line<'_>, width: usize, trim: bool) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut current = Line {
        spans: Vec::new(),
        style: line.style,
    };
    let mut current_width = 0usize;

    for span in &line.spans {
        let span_style = span.style;
        for symbol in terminal_symbols(&span.content) {
            let symbol_width = symbol.width as usize;
            let is_whitespace = symbol.text.chars().next().is_some_and(char::is_whitespace);
            if trim && current_width == 0 && symbol_width > 0 && is_whitespace {
                continue;
            }
            if symbol_width > 0 && current_width > 0 && current_width + symbol_width > width {
                finish_wrapped_line(&mut lines, &mut current, trim, line.style);
                current_width = 0;
                if trim && is_whitespace {
                    continue;
                }
            }
            push_wrapped_symbol(&mut current, symbol.text, span_style);
            current_width += symbol_width;
        }
    }

    finish_wrapped_line(&mut lines, &mut current, trim, line.style);
    if lines.is_empty() {
        lines.push(Line {
            spans: Vec::new(),
            style: line.style,
        });
    }
    lines
}

fn push_wrapped_symbol(line: &mut Line<'static>, symbol: &str, style: Style) {
    if let Some(last) = line.spans.last_mut()
        && last.style == style
    {
        last.content.to_mut().push_str(symbol);
        return;
    }
    line.spans.push(Span::styled(symbol.to_string(), style));
}

fn finish_wrapped_line(
    lines: &mut Vec<Line<'static>>,
    current: &mut Line<'static>,
    trim: bool,
    style: Style,
) {
    if trim {
        while let Some(last) = current.spans.last_mut() {
            let trimmed_len = last.content.trim_end_matches(char::is_whitespace).len();
            last.content.to_mut().truncate(trimmed_len);
            if last.content.is_empty() {
                current.spans.pop();
            } else {
                break;
            }
        }
    }
    lines.push(std::mem::take(current));
    current.style = style;
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
    /// Mutable access for span recycling between frames.
    pub fn content_mut(&mut self) -> &mut Line<'a> {
        &mut self.content
    }

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
    /// Mutable access for span recycling between frames.
    pub fn cells_mut(&mut self) -> &mut [Cell<'a>] {
        &mut self.cells
    }

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
    /// Pre-resolved column widths; when present (and non-empty), `render`
    /// skips re-running the constraint math so the caller's layout is used
    /// verbatim.
    resolved_widths: Option<Vec<u16>>,
    column_spacing: u16,
    style: Style,
    row_highlight_style: Style,
    highlight_symbol: Option<&'a str>,
}

impl<'a> Table<'a> {
    pub fn new<R: IntoIterator<Item = Row<'a>>, C: Into<Vec<Constraint>>>(
        rows: R,
        widths: C,
    ) -> Self {
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

    /// Provide already-resolved per-column widths (e.g. from the same layout
    /// pass that produced click regions). Takes precedence over `widths`
    /// during render when non-empty.
    pub fn column_widths_resolved(mut self, widths: Vec<u16>) -> Self {
        self.resolved_widths = Some(widths);
        self
    }

    /// Mutable access for span recycling between frames.
    pub fn header_mut(&mut self) -> Option<&mut Row<'a>> {
        self.header.as_mut()
    }

    /// Mutable access for span recycling between frames.
    pub fn rows_mut(&mut self) -> &mut [Row<'a>] {
        &mut self.rows
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
        resolve_column_widths(&self.widths, self.column_spacing, max_width)
    }
}

/// Resolve column constraints to widths across `max_width`, less the
/// spacing between columns (how `Table` lays out its columns).
pub fn resolve_column_widths(
    constraints: &[Constraint],
    column_spacing: u16,
    max_width: u16,
) -> Vec<u16> {
    if constraints.is_empty() {
        return vec![];
    }

    let spacing_total = column_spacing * (constraints.len().saturating_sub(1)) as u16;
    let available = max_width.saturating_sub(spacing_total) as i32;

    let mut widths: Vec<i32> = vec![0; constraints.len()];
    let mut remaining = available;
    let mut flex_count = 0;

    // First pass: fixed sizes (Length, Percentage, Ratio, Max)
    // Min and Fill are flexible - they start at minimum and can grow
    for (i, constraint) in constraints.iter().enumerate() {
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
        let mut extra = remaining % flex_count;
        for (i, constraint) in constraints.iter().enumerate() {
            match constraint {
                Constraint::Min(_) | Constraint::Fill(_) => {
                    widths[i] += per_flex;
                    if extra > 0 {
                        widths[i] += 1;
                        extra -= 1;
                    }
                }
                _ => {}
            }
        }
    } else if remaining > 0 {
        // All columns fixed: hand floor-division loss to the last column so
        // the table uses the full width.
        if let Some(last) = widths.last_mut() {
            *last += remaining;
        }
    }

    widths.into_iter().map(|w| w.max(0) as u16).collect()
}

impl Table<'_> {
    /// Shared-reference render so callers can keep ownership of `rows` and
    /// recycle their span strings across frames.
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
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

        // Apply base style to entire table area.
        buf.set_style(table_area, self.style);

        let col_widths = match self.resolved_widths {
            Some(ref w) if !w.is_empty() => w.clone(),
            _ => self.get_column_widths(table_area.width),
        };
        let mut y = table_area.y;

        // Render header
        if let Some(header) = &self.header
            && y < table_area.bottom()
        {
            // Apply header row style first
            let header_style = self.style.patch(header.style);
            buf.set_style(
                Rect::new(table_area.x, y, table_area.width, 1),
                header_style,
            );
            let mut x = table_area.x;
            for (i, cell) in header.cells.iter().enumerate() {
                if let Some(&width) = col_widths.get(i) {
                    // The row-wide set_style above already painted header_style
                    // across the full width (including inter-column gaps); only
                    // re-style this cell when it carries its own style override.
                    if cell.style != Style::default() {
                        let cell_style = header_style.patch(cell.style);
                        buf.set_style(Rect::new(x, y, width, 1), cell_style);
                    }
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
            // Apply row style first
            let row_style = self.style.patch(row.style);
            buf.set_style(Rect::new(table_area.x, y, table_area.width, 1), row_style);
            let mut x = table_area.x;
            for (i, cell) in row.cells.iter().enumerate() {
                if let Some(&width) = col_widths.get(i) {
                    // Row-wide set_style already applied row_style across the full
                    // width (incl. column-spacing gaps); skip the redundant per-cell
                    // restyle unless this cell carries its own style override.
                    if cell.style != Style::default() {
                        let cell_style = row_style.patch(cell.style);
                        buf.set_style(Rect::new(x, y, width, 1), cell_style);
                    }
                    buf.set_line(x, y, &cell.content, width);
                    x += width + self.column_spacing;
                }
            }
            y += row.height;
        }
    }
}

impl Widget for Table<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_ref(area, buf);
    }
}

impl Widget for &Table<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_ref(area, buf);
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

        // Apply base style to entire list area.
        buf.set_style(list_area, self.style);

        for (i, item) in self.items.iter().enumerate() {
            let y = list_area.y + i as u16;
            if y >= list_area.bottom() {
                break;
            }
            // Apply item style first, then render line with span styles.
            buf.set_style(
                Rect::new(list_area.x, y, list_area.width, 1),
                self.style.patch(item.style),
            );
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

        // Calculate thumb size and position. Clamp the position so a caller
        // passing position > scrollable pins the thumb to the track end
        // instead of pushing it off the track entirely.
        let viewport = state.viewport_content_length.max(1);
        let thumb_size = (track_len * viewport / state.content_length.max(1))
            .max(1)
            .min(track_len);
        let scrollable = state.content_length.saturating_sub(viewport);
        let max_pos = track_len - thumb_size; // safe: thumb_size is .min(track_len)
        let thumb_pos = (max_pos * state.position)
            .checked_div(scrollable)
            .unwrap_or(0)
            .min(max_pos);

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Console model for diff tests: printed symbols on a grid, where
    /// overwriting either half of a wide glyph erases the whole glyph, as real
    /// consoles do. Driven by the real `diff_frame` through `DiffSink`.
    struct Screen {
        width: usize,
        rows: Vec<Vec<(String, bool)>>, // (symbol, is_continuation)
        cursor: (usize, usize),
        prints: usize,
    }

    impl Screen {
        fn new(area: Rect) -> Self {
            Self {
                width: usize::from(area.width),
                rows: vec![
                    vec![(" ".to_string(), false); usize::from(area.width)];
                    usize::from(area.height)
                ],
                cursor: (0, 0),
                prints: 0,
            }
        }

        /// Blank the whole wide glyph covering `x` (no-op for a narrow cell).
        fn erase_glyph_at(&mut self, x: usize, y: usize) {
            let row = &mut self.rows[y];
            let mut lead = x;
            while lead > 0 && row[lead].1 {
                lead -= 1;
            }
            let mut end = lead + 1;
            while end < row.len() && row[end].1 {
                end += 1;
            }
            if end - lead > 1 {
                for cell in &mut row[lead..end] {
                    *cell = (" ".to_string(), false);
                }
            }
        }

        fn line(&self, y: usize) -> String {
            self.rows[y]
                .iter()
                .filter(|(_, continuation)| !continuation)
                .map(|(symbol, _)| symbol.as_str())
                .collect::<String>()
                .trim_end()
                .to_string()
        }
    }

    impl DiffSink for Screen {
        fn move_to(&mut self, x: u16, y: u16) -> io::Result<()> {
            self.cursor = (usize::from(x), usize::from(y));
            Ok(())
        }
        fn set_fg(&mut self, _: Color) -> io::Result<()> {
            Ok(())
        }
        fn set_bg(&mut self, _: Color) -> io::Result<()> {
            Ok(())
        }
        fn set_attribute(&mut self, _: Attribute) -> io::Result<()> {
            Ok(())
        }
        fn print(&mut self, symbol: &str) -> io::Result<()> {
            self.prints += 1;
            let (x, y) = self.cursor;
            let width = usize::from(terminal_symbol_width(symbol).max(1));
            if x >= self.width {
                return Ok(());
            }
            for column in x..(x + width).min(self.width) {
                self.erase_glyph_at(column, y);
            }
            self.rows[y][x] = (symbol.to_string(), false);
            for column in x + 1..(x + width).min(self.width) {
                self.rows[y][column] = (String::new(), true);
            }
            self.cursor.0 = x + width;
            Ok(())
        }
    }

    /// Visible text of a (normalized) buffer row: what a correct screen shows.
    fn buffer_line(buffer: &Buffer, y: u16) -> String {
        let mut normalized = buffer.clone();
        normalize_wide_row(&mut normalized, usize::from(y - buffer.area.y));
        (buffer.area.x..buffer.area.right())
            .filter_map(|x| normalized.get(x, y))
            .filter(|cell| !cell.is_continuation)
            .map(|cell| cell.symbol.as_str().to_string())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// Diff `back` onto the screen model and assert every row reads back.
    fn present(back: &mut Buffer, front: &mut Buffer, screen: &mut Screen) {
        let expected: Vec<String> = (back.area.y..back.area.bottom())
            .map(|y| buffer_line(back, y))
            .collect();
        diff_frame(back, front, screen).unwrap();
        for (y, line) in expected.iter().enumerate() {
            assert_eq!(&screen.line(y), line, "screen row {y} diverged");
            // `front` must mirror what the screen actually shows, or later
            // diffs skip cells that need repainting.
            let front_line: String = (front.area.x..front.area.right())
                .filter_map(|x| front.get(x, front.area.y + y as u16))
                .filter(|cell| !cell.is_continuation)
                .map(|cell| cell.symbol.as_str().to_string())
                .collect::<String>()
                .trim_end()
                .to_string();
            assert_eq!(
                &front_line, line,
                "front row {y} out of step with the screen"
            );
        }
    }

    #[test]
    fn graphemes_render_and_truncate_without_splitting_modifiers() {
        for (text, cluster, width) in [
            ("👍🏽A", "👍🏽", 2),
            ("🧑🏽‍💻A", "🧑🏽‍💻", 2),
            ("🇺🇸A", "🇺🇸", 2),
            ("éA", "é", 1),
        ] {
            let mut buf = Buffer::empty(Rect::new(0, 0, width + 1, 1));
            buf.set_string_truncated(0, 0, text, width + 1, Style::default());
            assert_eq!(buf.get(0, 0).unwrap().symbol.as_str(), cluster);
            assert_eq!(buf.get(width, 0).unwrap().symbol.as_str(), "A");
            let mut short = Buffer::empty(Rect::new(0, 0, width, 1));
            short.set_string_truncated(0, 0, text, width, Style::default());
            assert_eq!(short.get(0, 0).unwrap().symbol.as_str(), cluster);
            if width == 2 {
                assert!(short.get(1, 0).unwrap().is_continuation);
            }
        }
    }

    #[test]
    fn diff_replay_replaces_modifier_cluster_without_leaving_continuations() {
        let area = Rect::new(0, 0, 8, 1);
        let mut front = Buffer::empty(area);
        let mut screen = Screen::new(area);
        for text in ["👍🏽A", "🇺🇸B", "éC", "🧑🏽‍💻D", "normal"] {
            let mut back = Buffer::empty(area);
            back.set_string_truncated(0, 0, text, 8, Style::default());
            present(&mut back, &mut front, &mut screen);
            assert_eq!(screen.line(0), text);
        }
    }

    #[test]
    fn set_line_places_emoji_presentation_as_wide_symbol() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));

        buf.set_line(0, 0, &Line::raw("🛡️ System"), 12);

        assert_eq!(buf.get(0, 0).unwrap().symbol, "🛡️");
        assert!(buf.get(1, 0).unwrap().is_continuation);
        assert_eq!(buf.get(2, 0).unwrap().symbol, " ");
        assert_eq!(buf.get(3, 0).unwrap().symbol, "S");
        assert_eq!(buf.get(4, 0).unwrap().symbol, "y");
        assert_eq!(buf.get(5, 0).unwrap().symbol, "s");
        assert_eq!(buf.get(8, 0).unwrap().symbol, "m");
    }

    #[test]
    fn diff_replay_keeps_system_s_after_wide_indicator() {
        let area = Rect::new(0, 0, 24, 1);
        let mut front = Buffer::empty(area);
        let mut screen = Screen::new(area);

        let mut previous = Buffer::empty(area);
        previous.set_string(0, 0, "xxABsD", Style::default());
        present(&mut previous, &mut front, &mut screen);
        let mut current = Buffer::empty(area);
        current.set_line(0, 0, &Line::raw("🛡️ System"), area.width);
        present(&mut current, &mut front, &mut screen);

        assert_eq!(screen.line(0), "🛡️ System");
        assert!(!screen.line(0).contains("Sytem"));
    }

    #[test]
    fn test_scrollbar_thumb_clamped_to_track() {
        // A position far beyond the scrollable range must pin the thumb to
        // the track end, not push it off the track entirely (regression:
        // dialogs pass unclamped scroll offsets).
        let area = Rect::new(0, 0, 1, 10);
        let mut buf = Buffer::empty(area);
        let mut state = ScrollbarState::new(90)
            .position(500)
            .viewport_content_length(10);
        Scrollbar::new(ScrollbarOrientation::VerticalRight).render(area, &mut buf, &mut state);

        let thumb = "█";
        // The thumb is pinned to the bottom of the track...
        assert_eq!(buf.get(0, 9).unwrap().symbol, thumb);
        // ...and did not vanish into (or flood) the rest of the track.
        assert_ne!(buf.get(0, 0).unwrap().symbol, thumb);
    }

    #[test]
    fn test_scrollbar_thumb_proportional_to_viewport() {
        // With a viewport covering 1/4 of the content, the thumb should be
        // ~1/4 of the track and sit at the top when scrolled to position 0.
        // Regression: dialogs that omit viewport_content_length collapse the
        // thumb to a single cell regardless of how much content is visible.
        let area = Rect::new(0, 0, 1, 12);
        let mut buf = Buffer::empty(area);
        let mut state = ScrollbarState::new(40)
            .viewport_content_length(10)
            .position(0);
        Scrollbar::new(ScrollbarOrientation::VerticalRight).render(area, &mut buf, &mut state);

        let thumb = "█";
        let thumb_cells = (0..12)
            .filter(|&y| buf.get(0, y).unwrap().symbol == thumb)
            .count();
        // 12 * 10 / 40 = 3 cells, not the degenerate 1-cell thumb.
        assert_eq!(thumb_cells, 3);
        // At position 0 the thumb starts at the top of the track.
        assert_eq!(buf.get(0, 0).unwrap().symbol, thumb);
        assert_ne!(buf.get(0, 11).unwrap().symbol, thumb);
    }

    #[test]
    fn layout_all_fixed_constraints_cover_the_parent_area() {
        // Regression: with only fixed constraints the floor-division loss was
        // dropped entirely, leaving a dead background strip along the edge
        // (e.g. the CPU meter grid of Ratio(1, cols) columns).
        let area = Rect::new(0, 0, 101, 10);
        let rects = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(area);
        let total: u16 = rects.iter().map(|r| r.width).sum();
        assert_eq!(total, 101);

        // Same property when the loss is larger than one cell.
        let rects = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(area);
        let total: u16 = rects.iter().map(|r| r.width).sum();
        assert_eq!(total, 101);
        // Children tile the parent without gaps.
        let mut x = 0;
        for rect in &rects {
            assert_eq!(rect.x, x);
            x += rect.width;
        }
    }

    #[test]
    fn layout_flex_elements_absorb_remainder() {
        // Regression: remaining % flex_count cells were discarded, so two flex
        // columns over an odd width came up one cell short of the parent.
        let area = Rect::new(0, 0, 25, 5);
        let rects = Layout::horizontal([Constraint::Min(10), Constraint::Min(10)]).split(area);
        let total: u16 = rects.iter().map(|r| r.width).sum();
        assert_eq!(total, 25);
        // Each flex element keeps at least its minimum.
        for rect in &rects {
            assert!(rect.width >= 10);
        }
    }

    #[test]
    fn table_column_widths_use_the_full_width() {
        // Regression: same remainder-dropping algorithm as Layout::split left
        // the last table columns unused on non-divisible widths.
        let table = Table::new(Vec::<Row>::new(), [Constraint::Ratio(1, 3); 3]).column_spacing(0);
        let widths = table.get_column_widths(101);
        assert_eq!(widths.iter().sum::<u16>(), 101);
    }

    #[test]
    fn table_flex_columns_absorb_remainder() {
        let table = Table::new(
            Vec::<Row>::new(),
            [Constraint::Min(10), Constraint::Min(10)],
        )
        .column_spacing(0);
        let widths = table.get_column_widths(25);
        assert_eq!(widths.iter().sum::<u16>(), 25);
        assert!(widths.iter().all(|&w| w >= 10));
    }

    #[test]
    fn symbol_inline_roundtrip_ascii_box_drawing_and_wide() {
        // Every glyph the UI draws cell-by-cell must survive set_symbol
        // byte-for-byte while staying inside the cell (no heap allocation).
        for text in ["A", " ", "0", "─", "│", "┌", "█", "░", "日", "🛡️", "🇺🇸"]
        {
            let mut cell = BufferCell::default();
            cell.set_symbol(text);
            assert_eq!(&*cell.symbol, text, "roundtrip failed for {text:?}");
            assert_eq!(cell.symbol.as_str(), text);
            assert!(
                cell.symbol.is_inline(),
                "{text:?} ({} bytes) should stay inline",
                text.len()
            );
            assert_eq!(format!("{}", cell.symbol), text);
        }
    }

    #[test]
    fn symbol_long_grapheme_spills_to_heap_and_releases_on_clear() {
        // ZWJ family emoji is 25 UTF-8 bytes: too big for inline storage.
        let long = "👨\u{200d}👩\u{200d}👧\u{200d}👦";
        let mut cell = BufferCell::default();
        cell.set_symbol(long);
        assert_eq!(cell.symbol.as_str(), long);
        assert!(
            !cell.symbol.is_inline(),
            "25-byte grapheme must spill to heap"
        );

        // Clearing a reused cell must drop the box again.
        cell.reset();
        assert!(cell.symbol.is_inline());
        assert_eq!(cell.symbol.as_str(), " ");
    }

    #[test]
    fn symbol_append_spills_only_when_inline_capacity_exceeded() {
        let mut symbol = Symbol::default();
        symbol.clear(); // default is one space; start truly empty
        symbol.push_sanitized_str("abcde");
        assert!(symbol.is_inline());
        // 5 + 11 = 16 bytes: one past the inline cap.
        symbol.push_sanitized_str("fghijklmnopq");
        assert_eq!(symbol.as_str(), "abcdefghijklmnopq");
        assert!(!symbol.is_inline());

        // Appending still works on the spilled representation.
        symbol.push_sanitized_str("r");
        assert_eq!(symbol.as_str(), "abcdefghijklmnopqr");
    }

    #[test]
    fn symbol_sanitization_matches_previous_string_behavior() {
        let cases = [
            ("\t", " "),
            ("a\tb", "a b"),
            ("\u{1b}", "\u{FFFD}"), // ESC must never reach the terminal
            ("\u{85}", "\u{FFFD}"), // C1 control
            ("\u{7}", "\u{FFFD}"),  // BEL
            ("ok", "ok"),
        ];
        for (input, expected) in cases {
            let mut cell = BufferCell::default();
            cell.set_symbol(input);
            assert_eq!(cell.symbol.as_str(), expected, "input {input:?}");

            // set_char maps one char at a time, so it only applies to the
            // single-character cases.
            if input.chars().count() == 1 {
                let mut cell = BufferCell::default();
                cell.set_char(input.chars().next().unwrap());
                assert_eq!(cell.symbol.as_str(), expected, "set_char {input:?}");
            }
        }
    }

    #[test]
    fn reused_cell_equals_fresh_default_cell() {
        // flush_diff skips cells that compare equal to the previous frame, so
        // a reset cell must compare equal to a pristine one even after it held
        // a spilled heap symbol with stale bytes behind it.
        let mut used = BufferCell::default();
        used.set_symbol("👨\u{200d}👩\u{200d}👧\u{200d}👦");
        used.set_style(Style::default().fg(Color::Red).bg(Color::Blue));
        used.reset();

        assert_eq!(used, BufferCell::default());
    }

    #[test]
    fn symbol_size_stays_within_old_string_footprint() {
        // The whole point of the inline layout: no growth per cell versus the
        // previous `symbol: String` field.
        assert_eq!(std::mem::size_of::<Symbol>(), 16);
        assert_eq!(
            std::mem::size_of::<BufferCell>(),
            std::mem::size_of::<Symbol>() + 16 // fg + bg + modifier + continuation + padding
        );
        assert!(std::mem::size_of::<BufferCell>() <= 32);
    }

    #[test]
    fn buffer_resize_reuses_allocation_and_resets_cells() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10)); // 400 cells
        buf.set_string(0, 0, "hello 🛡️ world", Style::default());
        buf.set_style(buf.area, Style::default().fg(Color::Red));
        let cap_before = buf.content.capacity();

        // Shrink: the backing store must neither be freed nor re-allocated.
        buf.resize(Rect::new(0, 0, 20, 10));
        assert_eq!(buf.area, Rect::new(0, 0, 20, 10));
        assert_eq!(buf.content.len(), 200);
        assert_eq!(buf.content.capacity(), cap_before);
        assert!(
            buf.content
                .iter()
                .all(|cell| *cell == BufferCell::default()),
            "resize must leave every cell pristine, like Buffer::empty did"
        );

        // Growing back to the original size still does not reallocate.
        buf.resize(Rect::new(0, 0, 40, 10));
        assert_eq!(buf.content.len(), 400);
        assert_eq!(buf.content.capacity(), cap_before);
    }

    #[test]
    fn buffer_resize_wipes_content_so_diff_does_not_skip_rows() {
        // Regression guard for the resize path in Terminal::draw: after a
        // resize the screen is cleared, so a previous buffer that kept stale
        // content would make flush_diff skip rows that are now blank.
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 4));
        for x in 0..8 {
            buf.get_mut(x, 1).unwrap().set_symbol("█");
        }

        buf.resize(Rect::new(0, 0, 16, 3));

        let pristine = Buffer::empty(buf.area);
        assert_eq!(buf.content, pristine.content);
    }

    #[test]
    fn cells_hold_no_heap_symbols_after_full_reset_cycle() {
        // Draw text into every cell of a small buffer (some symbols spill),
        // then reset like Terminal::draw does each frame: all storage must be
        // back inline afterwards.
        let area = Rect::new(0, 0, 6, 2);
        let mut buf = Buffer::empty(area);
        buf.set_string(0, 0, "🛡️👨‍👩‍👧‍👦ok", Style::default());
        assert!(buf.content.iter().any(|c| !c.symbol.is_inline()));

        for cell in &mut buf.content {
            cell.reset();
        }
        assert!(buf.content.iter().all(|c| c.symbol.is_inline()));
        assert!(buf.content.iter().all(|c| *c == BufferCell::default()));
    }

    #[test]
    fn default_cell_constant_matches_default_and_reset() {
        assert_eq!(DEFAULT_CELL, BufferCell::default());
        let mut cell = BufferCell::default();
        cell.set_symbol("界");
        cell.reset();
        assert_eq!(cell, DEFAULT_CELL);
    }

    #[test]
    fn inline_clone_is_equal_and_heap_clone_still_deep_copies() {
        let mut inline = BufferCell::default();
        inline.set_symbol("é");
        assert_eq!(inline.clone(), inline);
        let mut heap = BufferCell::default();
        heap.set_symbol("👨\u{200d}👩\u{200d}👧\u{200d}👦");
        let copy = heap.clone();
        drop(heap);
        assert_eq!(copy.symbol.as_str(), "👨\u{200d}👩\u{200d}👧\u{200d}👦");
    }

    /// The ASCII fast path must produce exactly what the grapheme path did:
    /// tab → space, other controls → U+FFFD, one width-1 cell per byte.
    #[test]
    fn ascii_fast_path_sanitizes_like_the_grapheme_path() {
        let text = "a\tb\x01c\x7f\r\n";
        let expected = [
            "a", " ", "b", "\u{FFFD}", "c", "\u{FFFD}", "\u{FFFD}", "\u{FFFD}",
        ];
        let style = Style::default().fg(Color::Green);

        let mut via_string = Buffer::empty(Rect::new(0, 0, 10, 1));
        via_string.set_string(0, 0, text, style);
        let mut via_line = Buffer::empty(Rect::new(0, 0, 10, 1));
        via_line.set_line(0, 0, &Line::from(vec![Span::styled(text, style)]), 10);

        for buf in [&via_string, &via_line] {
            for (x, symbol) in expected.iter().enumerate() {
                let cell = buf.get(x as u16, 0).unwrap();
                assert_eq!(cell.symbol.as_str(), *symbol, "cell {x}");
                assert_eq!(cell.fg, Color::Green);
                assert!(!cell.is_continuation);
            }
        }
        // Truncation stops at the width limit.
        let mut short = Buffer::empty(Rect::new(0, 0, 10, 1));
        short.set_string_truncated(0, 0, "abcdef", 3, Style::default());
        assert_eq!(short.get(2, 0).unwrap().symbol.as_str(), "c");
        assert_eq!(short.get(3, 0).unwrap().symbol.as_str(), " ");
    }

    #[test]
    fn zero_width_mark_after_an_ascii_span_joins_its_last_cell() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        let line = Line::from(vec![Span::raw("ab"), Span::raw("\u{301}c")]);
        buf.set_line(0, 0, &line, 10);
        assert_eq!(buf.get(1, 0).unwrap().symbol.as_str(), "b\u{301}");
        assert_eq!(buf.get(2, 0).unwrap().symbol.as_str(), "c");
    }

    #[test]
    fn ascii_write_over_a_heap_symbol_frees_and_replaces_it() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        buf.set_string(0, 0, "👨\u{200d}👩\u{200d}👧\u{200d}👦", Style::default());
        buf.set_string(0, 0, "x", Style::default());
        let cell = buf.get(0, 0).unwrap();
        assert_eq!(cell.symbol.as_str(), "x");
        assert!(cell.symbol.is_inline());
    }

    // ===== Retained compositor =====

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 12,
        height: 3,
    };

    /// Paint `rows[y]` on row y with `base`, leaving other rows unclaimed.
    fn paint_rows(frame: &mut Frame, background: Color, base: Style, rows: &[&str]) {
        frame.begin(background);
        for (y, text) in rows.iter().enumerate() {
            let line = Line::raw(*text);
            frame.paint_line(Rect::new(0, y as u16, AREA.width, 1), &line, base);
        }
        frame.finish_base();
    }

    #[test]
    fn unchanged_rows_are_not_repainted() {
        let mut compositor = Compositor::new(AREA);
        compositor.draw(AREA, |f| {
            paint_rows(f, Color::Black, Style::default(), &["ab", "cd"])
        });
        assert_eq!(
            compositor.buffer().dirty_rows(),
            3,
            "first frame paints everything"
        );
        compositor.acknowledge();

        compositor.draw(AREA, |f| {
            paint_rows(f, Color::Black, Style::default(), &["ab", "cd"])
        });
        assert_eq!(compositor.buffer().dirty_rows(), 0);

        compositor.draw(AREA, |f| {
            paint_rows(f, Color::Black, Style::default(), &["ab", "cX"])
        });
        assert_eq!(compositor.buffer().dirty_rows(), 1);
        assert_eq!(compositor.buffer().get(1, 1).unwrap().symbol.as_str(), "X");
    }

    #[test]
    fn a_repaint_resets_the_row_instead_of_layering_on_it() {
        let mut compositor = Compositor::new(AREA);
        let bold = Style::default().add_modifier(Modifier::BOLD).fg(Color::Red);
        compositor.draw(AREA, |f| paint_rows(f, Color::Black, bold, &["long text"]));
        compositor.draw(AREA, |f| {
            paint_rows(f, Color::Black, Style::default(), &["ab"])
        });

        let mut reference = Buffer::empty(AREA);
        paint_rows(
            &mut Frame::new(&mut reference),
            Color::Black,
            Style::default(),
            &["ab"],
        );
        assert_eq!(compositor.buffer().content, reference.content);
        assert!(
            compositor
                .buffer()
                .content
                .iter()
                .all(|c| c.modifier.is_empty())
        );
    }

    #[test]
    fn a_background_change_repaints_every_row() {
        let mut compositor = Compositor::new(AREA);
        compositor.draw(AREA, |f| {
            paint_rows(f, Color::Black, Style::default(), &["ab"])
        });
        compositor.acknowledge();
        compositor.draw(AREA, |f| {
            paint_rows(f, Color::Blue, Style::default(), &["ab"])
        });
        assert_eq!(compositor.buffer().dirty_rows(), 3);
        assert!(
            compositor
                .buffer()
                .content
                .iter()
                .all(|c| c.bg == Color::Blue)
        );
    }

    #[test]
    fn rows_under_an_overlay_repaint_once_it_closes() {
        let mut compositor = Compositor::new(AREA);
        compositor.draw(AREA, |f| {
            paint_rows(f, Color::Black, Style::default(), &["base0", "base1"]);
            f.render_widget(Paragraph::new("DIALOG"), Rect::new(0, 1, 6, 1));
        });
        assert_eq!(compositor.buffer().get(0, 1).unwrap().symbol.as_str(), "D");
        compositor.acknowledge();

        compositor.draw(AREA, |f| {
            paint_rows(f, Color::Black, Style::default(), &["base0", "base1"])
        });
        assert_eq!(compositor.buffer().dirty_rows(), 1, "only the overlay row");
        assert_eq!(compositor.buffer().get(0, 1).unwrap().symbol.as_str(), "b");
    }

    #[test]
    fn invalidate_marks_everything_for_repaint() {
        let mut compositor = Compositor::new(AREA);
        compositor.draw(AREA, |f| {
            paint_rows(f, Color::Black, Style::default(), &["ab"])
        });
        compositor.acknowledge();
        compositor.invalidate();
        compositor.draw(AREA, |f| {
            paint_rows(f, Color::Black, Style::default(), &["ab"])
        });
        assert_eq!(compositor.buffer().dirty_rows(), 3);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "outside Frame::paint_row"))]
    fn base_layer_writes_must_go_through_paint_row() {
        let mut compositor = Compositor::new(AREA);
        compositor.draw(AREA, |f| {
            f.begin(Color::Black);
            f.render_widget(Paragraph::new("stray"), Rect::new(0, 0, 5, 1));
            f.finish_base();
        });
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "painted twice"))]
    fn a_row_is_painted_once_per_frame() {
        let mut frame_buffer = Buffer::empty(AREA);
        let mut frame = Frame::new(&mut frame_buffer);
        frame.begin(Color::Black);
        let line = Line::raw("x");
        frame.paint_line(Rect::new(0, 0, 4, 1), &line, Style::default());
        frame.paint_line(Rect::new(4, 0, 4, 1), &line, Style::default());
    }

    #[test]
    fn diff_visits_only_dirty_rows_and_keeps_front_in_step() {
        let area = Rect::new(0, 0, 6, 3);
        let mut back = Buffer::empty(area);
        back.set_string(0, 0, "abc", Style::default());
        back.set_string(0, 2, "xyz", Style::default());
        let mut front = Buffer::empty(area);
        let mut screen = Screen::new(area);
        present(&mut back, &mut front, &mut screen);
        assert_eq!(front.content, back.content);
        assert_eq!(back.dirty_rows(), 0);

        let before = screen.prints;
        back.set_string(1, 2, "Y", Style::default());
        present(&mut back, &mut front, &mut screen);
        assert_eq!(screen.prints - before, 1, "one changed cell, one print");
        assert!(!diff_frame(&mut back, &mut front, &mut screen).unwrap());
    }

    /// A dialog edge landing on the right half of a wide glyph used to leave
    /// its border character on screen after the dialog closed.
    #[test]
    fn fresh_row_paint_matches_reset_style_and_line_writes() {
        // paint_fresh_row must leave exactly the cells and flags of the
        // sequence it replaces (the retained-render gate cannot tell: both
        // of its sides paint fresh rows the same way).
        let mut state = 0x853C_49E6_748F_EA9Bu64;
        let mut next = |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        let texts = [
            "abc",
            "PID",
            "\t",
            "\u{7}",
            "🛡️",
            "進程.exe",
            "e\u{301}x",
            "",
            "│ ",
            "🌿",
            "C:\\Windows\\System32\\svchost.exe -k netsvcs",
            " ",
            "\u{301}",
        ];
        let colors = [
            None,
            Some(Color::Red),
            Some(Color::Rgb(1, 2, 3)),
            Some(Color::Indexed(99)),
            Some(Color::Reset),
        ];
        let style = |next: &mut dyn FnMut(u64) -> u64| {
            let mut style = Style {
                fg: colors[next(5) as usize],
                bg: colors[next(5) as usize],
                ..Style::default()
            };
            if next(3) == 0 {
                style = style.add_modifier(Modifier::BOLD);
            }
            if next(4) == 0 {
                style = style.remove_modifier(Modifier::BOLD | Modifier::DIM);
            }
            style
        };
        let area = Rect::new(3, 1, 40, 3);
        for case in 0..3000 {
            let lines: Vec<Line<'static>> = (0..1 + next(4))
                .map(|_| {
                    let spans: Vec<Span<'static>> = (0..next(4))
                        .map(|_| {
                            Span::styled(texts[next(texts.len() as u64) as usize], style(&mut next))
                        })
                        .collect();
                    Line::from(spans).style(style(&mut next))
                })
                .collect();
            let segs: Vec<RowSeg<'_>> = lines
                .iter()
                .map(|line| RowSeg::line(next(50) as u16, next(30) as u16, style(&mut next), line))
                .collect();
            let spec = RowSpec {
                x: next(48) as u16,
                width: next(48) as u16,
                base: style(&mut next),
                segs: &segs,
            };
            let background = colors[next(5) as usize].unwrap_or(Color::Blue);
            let y = area.y + next(3) as u16;
            let row = usize::from(y - area.y);

            // Both start from the same garbage: heap symbols, wide glyphs,
            // stray continuations, styles.
            let mut garbage = Buffer::empty(area);
            for x in area.x..area.right() {
                let text = texts[next(texts.len() as u64) as usize];
                garbage.set_string(x, y, text, style(&mut next));
            }
            garbage.row_flags.fill(0);
            let mut expected = garbage.clone();
            let mut actual = garbage;

            expected.reset_row(row, &blank_cell(background));
            expected.set_style(Rect::new(spec.x, y, spec.width, 1), spec.base);
            for seg in spec.segs {
                if seg.style != Style::default() {
                    expected.set_style(
                        Rect::new(seg.x, y, seg.width, 1),
                        spec.base.patch(seg.style),
                    );
                }
                expected.set_spans(seg.x, y, seg.spans, seg.line_style, seg.width);
            }
            actual.paint_fresh_row(row, background, &spec);

            assert_eq!(actual.content, expected.content, "case {case}");
            assert_eq!(
                actual.row_flags[row] & ROW_DIRTY,
                expected.row_flags[row] & ROW_DIRTY,
                "case {case}"
            );
        }
    }

    #[test]
    fn overlay_over_half_a_wide_glyph_leaves_no_remnant() {
        let area = Rect::new(0, 0, 12, 1);
        let mut compositor = Compositor::new(area);
        let mut front = Buffer::empty(area);
        let mut screen = Screen::new(area);
        let base = |f: &mut Frame| {
            f.begin(Color::Reset);
            let line = Line::raw("🛡️ System");
            f.paint_line(area, &line, Style::default());
            f.finish_base();
        };

        compositor.draw(area, base);
        present(&mut compositor.back, &mut front, &mut screen);
        assert_eq!(screen.line(0), "🛡️ System");

        // Border at x=1: the shield's continuation cell.
        compositor.draw(area, |f| {
            base(f);
            if let Some(cell) = f.buffer_mut().get_mut(1, 0) {
                cell.set_symbol("│");
                cell.is_continuation = false;
            }
        });
        present(&mut compositor.back, &mut front, &mut screen);
        assert_eq!(screen.line(0), " │ System");

        compositor.draw(area, base);
        present(&mut compositor.back, &mut front, &mut screen);
        assert_eq!(screen.line(0), "🛡️ System");
    }

    #[test]
    fn clear_then_redraw_resends_retained_rows() {
        let area = Rect::new(0, 0, 8, 2);
        let mut compositor = Compositor::new(area);
        let mut front = Buffer::empty(area);
        let mut screen = Screen::new(area);
        let draw = |c: &mut Compositor| {
            c.draw(area, |f| {
                paint_rows(f, Color::Reset, Style::default(), &["keep", "me"])
            });
        };
        draw(&mut compositor);
        present(&mut compositor.back, &mut front, &mut screen);

        // What Terminal::clear does: the console is wiped underneath us.
        screen = Screen::new(area);
        front.resize(area);
        compositor.invalidate();
        draw(&mut compositor);
        present(&mut compositor.back, &mut front, &mut screen);
        assert_eq!(screen.line(0), "keep");
        assert_eq!(screen.line(1), "me");
    }

    #[test]
    fn packed_colors_are_distinct_and_never_the_no_color_marker() {
        let mut colors = vec![
            Color::Reset,
            Color::Black,
            Color::Red,
            Color::Green,
            Color::Yellow,
            Color::Blue,
            Color::Magenta,
            Color::Cyan,
            Color::Gray,
            Color::DarkGray,
            Color::LightRed,
            Color::LightGreen,
            Color::LightYellow,
            Color::LightBlue,
            Color::LightMagenta,
            Color::LightCyan,
            Color::White,
        ];
        colors.extend([0u8, 1, 16, 255].map(Color::Indexed));
        colors.extend(
            [(0, 0, 0), (0, 0, 16), (255, 255, 255), (1, 2, 3)]
                .map(|(r, g, b)| Color::Rgb(r, g, b)),
        );
        let packed: std::collections::HashSet<u32> = colors.iter().map(|c| c.packed()).collect();
        assert_eq!(packed.len(), colors.len());
        assert!(!packed.contains(&u32::MAX));
    }
}
