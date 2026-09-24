use crate::terminal::{Frame, Modifier, Rect, RowSeg, RowSpec, Span, Style};
use std::cell::RefCell;
use std::collections::VecDeque;

use super::text_pool::{pooled_fmt, pooled_str, pooled_string, recycle_spans, recycle_vec};
use crate::app::{App, UIElement, UIRegion};
use crate::config::MeterMode;
use crate::numfmt::{push_round0, push_tenths};
use crate::system::push_bytes;

/// Pre-computed bar strings to avoid repeated String::repeat() allocations.
/// Maximum bar width is 128 characters which covers most terminal widths.
const MAX_BAR_WIDTH: usize = 128;

/// Minimum cap for CPU/Mem/Swap bar display width. Bars never render
/// shorter than col_width suggests, but also never balloon past
/// `max_bar_width(col_width)`. See `max_bar_width` for the scaling curve.
const BAR_CAP_FLOOR: usize = 40;
/// Hard absolute cap so even on ultrawide monitors bars stay readable
/// (a 200-char bar is just a blur).
const BAR_CAP_CEIL: usize = 72;

/// Adaptive bar-width cap.
///
/// - At typical widths the cap is flat at [`BAR_CAP_FLOOR`], which smooths the
///   bar-size pop when the layout drops a meter column (e.g. 3 cols @ width 150
///   → 2 cols @ width 149).
/// - On wide columns the cap grows to about two-thirds of the column so bars
///   don't leave a huge blank gap between `]` and the column's right edge.
/// - Beyond [`BAR_CAP_CEIL`] the cap plateaus again — past a certain point a
///   longer bar stops conveying more information.
#[inline]
fn max_bar_width(col_width: usize) -> usize {
    (col_width * 2 / 3).clamp(BAR_CAP_FLOOR, BAR_CAP_CEIL)
}

/// Pre-computed string of '|' characters for bar fills
static BAR_FILL: &str = "||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||||";

/// Pre-computed string of ' ' characters for bar empty space
static BAR_EMPTY: &str = "                                                                                                                                ";

/// Get a slice of bar fill characters (more efficient than String::repeat)
#[inline]
fn bar_fill(width: usize) -> &'static str {
    &BAR_FILL[..width.min(MAX_BAR_WIDTH)]
}

/// Get a slice of bar empty characters (more efficient than String::repeat)
#[inline]
fn bar_empty(width: usize) -> &'static str {
    &BAR_EMPTY[..width.min(MAX_BAR_WIDTH)]
}

/// Braille characters for sparkline graph - htop style
/// Each character encodes TWO data points (left column, right column)
/// Index = left_height * 5 + right_height (each 0-4 for 4 vertical dots)
/// This gives 25 combinations per character cell
const GRAPH_DOTS_UTF8: [&str; 25] = [
    /*00*/ " ", /*01*/ "⢀", /*02*/ "⢠", /*03*/ "⢰", /*04*/ "⢸",
    /*10*/ "⡀", /*11*/ "⣀", /*12*/ "⣠", /*13*/ "⣰", /*14*/ "⣸",
    /*20*/ "⡄", /*21*/ "⣄", /*22*/ "⣤", /*23*/ "⣴", /*24*/ "⣼",
    /*30*/ "⡆", /*31*/ "⣆", /*32*/ "⣦", /*33*/ "⣶", /*34*/ "⣾",
    /*40*/ "⡇", /*41*/ "⣇", /*42*/ "⣧", /*43*/ "⣷", /*44*/ "⣿",
];

/// Decide how many meter columns to display in the header based on terminal width.
///
/// Breakpoints are tuned so:
/// - Tiny terminals (<80) collapse to a single column so bars stay readable.
/// - Typical widths (80..150) use 2 columns — matches htop's default meter layout.
/// - Wide (150..220) unlocks 3 columns.
/// - Ultrawide (>=220) unlocks 4 columns so the header fills the screen without
///   leaving a huge blank band at the right edge.
///
/// Two additional rules keep the header tidy:
/// - Many-core machines (>16 CPUs) bump to at least 3 columns once width ≥ 100,
///   so the header never grows taller than ~6 rows on big servers.
/// - Column count never exceeds `cpu_count` — no point drawing empty columns.
fn calculate_meter_columns(width: u16, cpu_count: usize) -> usize {
    let base = if width < 80 {
        1
    } else if width < 150 {
        2
    } else if width < 220 {
        3
    } else {
        4
    };
    let many_core = if cpu_count > 16 && width >= 100 {
        base.max(3)
    } else {
        base
    };
    many_core.min(cpu_count.max(1))
}

/// Whether the GPU meter row is shown: requires a render-capable hardware
/// adapter to exist and the meter to be enabled in config. On machines
/// without a GPU the header layout is unchanged.
fn gpu_meter_visible(app: &App) -> bool {
    app.config.show_gpu_meter
        && app.config.gpu_meter_mode != MeterMode::Hidden
        && app.system_metrics.gpu.is_some()
}

/// Whether the NPU meter row is shown: requires an NPU (MCDM compute-only
/// adapter) to exist and the meter to be enabled in config. On machines
/// without an NPU the header layout is unchanged.
fn npu_meter_visible(app: &App) -> bool {
    app.config.show_npu_meter
        && app.config.npu_meter_mode != MeterMode::Hidden
        && app.system_metrics.npu.is_some()
}

fn visible_cpu_count(app: &App) -> usize {
    if app.config.show_cpu_meters && app.config.cpu_meter_mode != MeterMode::Hidden {
        app.system_metrics.cpu.core_usage.len()
    } else {
        0
    }
}

fn memory_meter_visible(app: &App) -> bool {
    app.config.show_memory_meter && app.config.memory_meter_mode != MeterMode::Hidden
}

fn swap_meter_visible(app: &App) -> bool {
    app.config.show_swap_meter && app.config.memory_meter_mode != MeterMode::Hidden
}

fn tasks_meter_visible(app: &App) -> bool {
    app.config.show_tasks_meter
}

fn uptime_meter_visible(app: &App) -> bool {
    app.config.show_uptime_meter
}

/// How many "extra" (non-CPU) meter rows a given column holds.
/// `left_extras` is Mem/Swap/GPU/NPU rows present in the left column.
/// `right_extras` is Tasks/Uptime rows present in the right column.
///
/// - Leftmost column gets Mem + Swp (2 rows), plus GPU/NPU when present.
/// - Rightmost column gets Tasks + Uptime (2 rows).
/// - On single-column mode the one column holds all (Mem, Swp, [GPU], [NPU], Tasks, Uptime).
/// - Middle columns (only when col_count == 3) have no mandatory extras —
///   Net/Dsk/Bat fill empty CPU slots opportunistically inside `draw_meter_column`.
fn extras_for_column(
    col_idx: usize,
    col_count: usize,
    left_extras: usize,
    right_extras: usize,
) -> usize {
    if col_count == 1 {
        left_extras + right_extras
    } else if col_idx == 0 {
        left_extras
    } else if col_idx == col_count - 1 {
        right_extras
    } else {
        0
    }
}

/// Whether a given column hosts Net/Dsk/Bat fillers in its empty CPU slots.
///
/// - 1 col:  no fillers (narrow collapsed view matches bare htop).
/// - 2 cols: rightmost column hosts fillers before Tasks/Uptime (historical).
/// - 3+ cols: every middle column (non-leftmost, non-rightmost) hosts fillers.
///
/// A shared cursor in `draw` walks left-to-right so the three fillers spread
/// across columns instead of stacking in one.
fn col_hosts_fillers(col_idx: usize, col_count: usize) -> bool {
    if col_count < 2 {
        return false;
    }
    if col_count == 2 {
        return col_idx == 1;
    }
    col_idx > 0 && col_idx < col_count - 1
}

/// Whether the Net / Dsk / Bat fillers render in the current layout, in fill
/// order. Computed from the same slot arithmetic the draw walk uses (empty
/// CPU slots in filler-hosting columns, left to right), so collection gating
/// (`App::canonical_collect_requirements`) can mirror render visibility
/// exactly instead of drifting from it.
pub(crate) fn filler_plan(app: &App) -> [bool; 3] {
    let mut plan = [false; 3];
    if !app.show_header {
        return plan;
    }
    let cpu_count = visible_cpu_count(app);
    let cols = calculate_meter_columns(app.terminal_width, cpu_count);
    if cols < 2 {
        // Single-column layout hosts no fillers.
        return plan;
    }
    let meter_rows = meter_rows_for(cpu_count, cols);
    let mut cursor = 0usize;
    for col_idx in 0..cols {
        if !col_hosts_fillers(col_idx, cols) {
            continue;
        }
        // CPUs assigned to this column (column-major: cpu_idx = row*cols+col).
        let shown = if cpu_count > col_idx {
            (((cpu_count - col_idx - 1) / cols) + 1).min(meter_rows)
        } else {
            0
        };
        for _ in 0..meter_rows.saturating_sub(shown) {
            if cursor < 3 {
                plan[cursor] = true;
            }
            cursor += 1;
        }
    }
    plan
}

/// Compute the meter-row count (CPU block height in each column).
/// Pads to min 4 on multi-column layouts so Net/Dsk/Bat fillers stay visible
/// even on low-CPU systems (matches htop-win's historical behavior).
fn meter_rows_for(cpu_count: usize, cols: usize) -> usize {
    let cpu_rows = cpu_count.div_ceil(cols.max(1));
    if cols == 1 { cpu_rows } else { cpu_rows.max(4) }
}

/// Calculate the header height based on CPU count and current terminal width.
pub fn calculate_header_height(app: &App) -> u16 {
    let cpu_count = visible_cpu_count(app);
    let cols = calculate_meter_columns(app.terminal_width, cpu_count);
    let meter_rows = meter_rows_for(cpu_count, cols);
    let left_extras = memory_meter_visible(app) as usize
        + swap_meter_visible(app) as usize
        + gpu_meter_visible(app) as usize
        + npu_meter_visible(app) as usize;
    let right_extras = tasks_meter_visible(app) as usize + uptime_meter_visible(app) as usize;
    let extras = if cols == 1 {
        left_extras + right_extras
    } else {
        left_extras.max(right_extras)
    };
    (meter_rows + extras) as u16
}

/// Replicates `Layout::split` for `cols` x `Constraint::Ratio(1, cols)`
/// horizontal, spacing 0: every column gets `width / cols`, and the
/// floor-division remainder goes to the last column (Layout's all-fixed path).
fn ratio_column_rect(inner: Rect, col_idx: usize, cols: usize) -> Rect {
    // Layout::split bails out with default rects for an empty area.
    if inner.is_empty() {
        return Rect::default();
    }
    let base = inner.width as usize / cols;
    let rem = inner.width as usize % cols;
    let width = if col_idx + 1 == cols { base + rem } else { base };
    Rect::new(
        inner.x + (base * col_idx) as u16,
        inner.y,
        width as u16,
        inner.height,
    )
}

/// Replicates `Layout::split` for `n` x `Constraint::Length(1)` vertical,
/// spacing 0: rows past the available height collapse to 0, and the last row
/// absorbs the leftover height (Layout's all-fixed path).
fn length_row_rect(area: Rect, row_idx: usize, n: usize) -> Rect {
    // Layout::split bails out with default rects for an empty area.
    if area.is_empty() {
        return Rect::default();
    }
    let h_avail = area.height as usize;
    let height = if row_idx + 1 == n {
        // 1 for the row itself plus any leftover, saturating to 0 past the end
        h_avail.saturating_sub(n - 1)
    } else {
        usize::from(row_idx < h_avail)
    };
    // y accumulates the computed heights of all preceding rows: 1 each until
    // the available height runs out, then 0.
    Rect::new(
        area.x,
        area.y + row_idx.min(h_avail) as u16,
        area.width,
        height as u16,
    )
}

/// Meter lines laid out for this frame: line `(rect, range)` is what a
/// one-line `Paragraph` of `spans[range]` renders in `rect`. Kept across
/// frames (see `SLOTS`), so the header builds its lines without allocating:
/// owned span text comes from the string pool and returns to it once painted.
struct Slots {
    spans: Vec<Span<'static>>,
    lines: Vec<(Rect, std::ops::Range<usize>)>,
    segs: Vec<RowSeg<'static>>,
}

impl Slots {
    /// Add one meter line in `area`.
    fn line<const N: usize>(&mut self, area: Rect, spans: [Span<'static>; N]) {
        let start = self.spans.len();
        self.spans.extend(spans);
        self.lines.push((area, start..self.spans.len()));
    }
}

thread_local! {
    static SLOTS: RefCell<Slots> = const {
        RefCell::new(Slots {
            spans: Vec::new(),
            lines: Vec::new(),
            segs: Vec::new(),
        })
    };
}

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    // The header needs no background pass of its own: the frame's base layer
    // is already the theme background.
    let inner = area;

    let cpu_count = visible_cpu_count(app);
    let cols = calculate_meter_columns(inner.width, cpu_count);
    let meter_rows = meter_rows_for(cpu_count, cols);

    // Shared cursor so Net/Dsk/Bat fillers spread across any middle columns
    // left-to-right instead of all landing in the first filler-hosting column.
    let plan = filler_plan(app);
    let mut filler_cursor = 0usize;
    SLOTS.with(|slots| {
        let slots = &mut *slots.borrow_mut();
        for col_idx in 0..cols {
            draw_meter_column(
                slots,
                app,
                ratio_column_rect(inner, col_idx, cols),
                col_idx,
                cols,
                meter_rows,
                &plan,
                &mut filler_cursor,
            );
        }
        paint_slots(frame, inner, slots);
    });
}

/// Paint the meter lines as one `paint_row` per screen row, so an unchanged
/// header row is skipped as a whole, then empty the slots for the next frame.
/// A one-line `Paragraph` renders nothing into an empty rect and only its
/// first row otherwise; the extra rows of a taller slot stay blank.
fn paint_slots(frame: &mut Frame, area: Rect, slots: &mut Slots) {
    slots.lines.retain(|(rect, _)| !rect.is_empty());
    // Positions are distinct, so an unstable sort gives the same order.
    slots
        .lines
        .sort_unstable_by_key(|(rect, _)| (rect.y, rect.x));
    let mut segs: Vec<RowSeg<'_>> = recycle_vec(std::mem::take(&mut slots.segs));
    for row in slots.lines.chunk_by(|a, b| a.0.y == b.0.y) {
        segs.clear();
        segs.extend(row.iter().map(|(rect, range)| RowSeg {
            x: rect.x,
            width: rect.width,
            style: Style::default(),
            line_style: Style::default(),
            spans: &slots.spans[range.clone()],
        }));
        frame.paint_row(
            row[0].0.y,
            &RowSpec {
                x: area.x,
                width: area.width,
                base: Style::default(),
                segs: &segs,
            },
        );
    }
    slots.segs = recycle_vec(segs);
    slots.lines.clear();
    recycle_spans(&mut slots.spans);
}

/// A pooled string holding `value` as `{:width$.1}` followed by `suffix`.
fn tenths_text(value: f32, width: usize, suffix: &str) -> String {
    let mut text = pooled_string();
    push_tenths(&mut text, value, width);
    text.push_str(suffix);
    text
}

/// A pooled string holding `prefix`, `bytes` as `format_bytes` shows them,
/// then `suffix`.
fn bytes_text(prefix: &str, bytes: u64, suffix: &str) -> String {
    let mut text = pooled_str(prefix);
    push_bytes(&mut text, bytes);
    text.push_str(suffix);
    text
}

/// A pooled string holding `used/total` as `format_bytes` shows them.
fn used_total_text(used: u64, total: u64) -> String {
    let mut text = bytes_text("", used, "/");
    push_bytes(&mut text, total);
    text
}

/// Draw a single meter column of the header.
///
/// CPUs are laid out column-major: `cpu_idx = row * col_count + col_idx`.
/// After the CPU block each column appends its role-specific extras (see
/// `extras_for_column`). In 3-column mode, the middle column has no mandatory
/// extras so its empty CPU slots are filled with Net / Dsk / Bat info.
#[allow(clippy::too_many_arguments)] // layout params are all distinct concerns
fn draw_meter_column(
    slots: &mut Slots,
    app: &mut App,
    area: Rect,
    col_idx: usize,
    col_count: usize,
    meter_rows: usize,
    filler_plan: &[bool; 3],
    filler_cursor: &mut usize,
) {
    let cpu_count = visible_cpu_count(app);
    let has_memory = memory_meter_visible(app);
    let has_swap = swap_meter_visible(app);
    let has_gpu = gpu_meter_visible(app);
    let has_npu = npu_meter_visible(app);
    let has_tasks = tasks_meter_visible(app);
    let has_uptime = uptime_meter_visible(app);
    let left_extras = has_memory as usize + has_swap as usize + has_gpu as usize + has_npu as usize;
    let right_extras = has_tasks as usize + has_uptime as usize;
    let extras = extras_for_column(col_idx, col_count, left_extras, right_extras);
    let total_rows = meter_rows + extras;
    if total_rows == 0 {
        return;
    }

    let hosts_fillers = col_hosts_fillers(col_idx, col_count);

    // CPU block (up to meter_rows). Empty CPU slots in filler-hosting columns
    // advance the shared filler cursor (Net → Dsk → Bat); empty slots in other
    // columns stay blank.
    for row_idx in 0..meter_rows {
        let row = length_row_rect(area, row_idx, total_rows);
        let cpu_idx = row_idx * col_count + col_idx;
        if cpu_idx < cpu_count {
            app.ui_bounds.add_region(UIRegion {
                element: UIElement::CpuMeter(Some(cpu_idx)),
                x: row.x,
                y: row.y,
                width: row.width,
                height: row.height,
            });
            draw_cpu_bar(
                slots,
                app,
                cpu_idx,
                app.system_metrics.cpu.core_usage[cpu_idx],
                row,
            );
        } else if hosts_fillers && *filler_cursor < 3 {
            // The plan decides whether this filler renders anywhere; a gated
            //-off filler leaves the slot blank on purpose.
            if filler_plan[*filler_cursor] {
                match *filler_cursor {
                    0 => draw_network_info(slots, app, row),
                    1 => draw_disk_info(slots, app, row),
                    2 => draw_battery_info(slots, app, row),
                    _ => {}
                }
            }
            *filler_cursor += 1;
        }
    }

    // Extras block — order depends on column role. Built in a fixed-size
    // array (max 6: Mem, Swp, [GPU], [NPU], Tasks, Uptime) to keep the
    // render path allocation-free.
    let is_leftmost = col_idx == 0;
    let is_rightmost = col_idx == col_count - 1;
    let mut order = [ExtraMeter::Memory; 6];
    let mut order_len = 0;
    if col_count == 1 || is_leftmost {
        if has_memory {
            order[order_len] = ExtraMeter::Memory;
            order_len += 1;
        }
        if has_swap {
            order[order_len] = ExtraMeter::Swap;
            order_len += 1;
        }
        if has_gpu {
            order[order_len] = ExtraMeter::Gpu;
            order_len += 1;
        }
        if has_npu {
            order[order_len] = ExtraMeter::Npu;
            order_len += 1;
        }
    }
    if col_count == 1 || is_rightmost {
        if has_tasks {
            order[order_len] = ExtraMeter::Tasks;
            order_len += 1;
        }
        if has_uptime {
            order[order_len] = ExtraMeter::Uptime;
            order_len += 1;
        }
    }

    for (extra_row, meter) in (meter_rows..).zip(order[..order_len].iter()) {
        if extra_row >= total_rows {
            break;
        }
        let row = length_row_rect(area, extra_row, total_rows);
        match meter {
            ExtraMeter::Memory => {
                app.ui_bounds.add_region(UIRegion {
                    element: UIElement::MemoryMeter,
                    x: row.x,
                    y: row.y,
                    width: row.width,
                    height: row.height,
                });
                draw_memory_bar(slots, app, row);
            }
            ExtraMeter::Swap => {
                app.ui_bounds.add_region(UIRegion {
                    element: UIElement::SwapMeter,
                    x: row.x,
                    y: row.y,
                    width: row.width,
                    height: row.height,
                });
                draw_swap_bar(slots, app, row);
            }
            ExtraMeter::Gpu => {
                app.ui_bounds.add_region(UIRegion {
                    element: UIElement::GpuMeter,
                    x: row.x,
                    y: row.y,
                    width: row.width,
                    height: row.height,
                });
                draw_gpu_bar(slots, app, row);
            }
            ExtraMeter::Npu => {
                app.ui_bounds.add_region(UIRegion {
                    element: UIElement::NpuMeter,
                    x: row.x,
                    y: row.y,
                    width: row.width,
                    height: row.height,
                });
                draw_npu_bar(slots, app, row);
            }
            ExtraMeter::Tasks => draw_tasks_info(slots, app, row),
            ExtraMeter::Uptime => draw_uptime_info(slots, app, row),
        }
    }
}

#[derive(Copy, Clone)]
enum ExtraMeter {
    Memory,
    Swap,
    Gpu,
    Npu,
    Tasks,
    Uptime,
}

fn draw_cpu_bar(slots: &mut Slots, app: &App, cpu_idx: usize, usage: f32, area: Rect) {
    let mode = app.config.cpu_meter_mode;

    // Hidden mode: don't render anything
    if mode == MeterMode::Hidden {
        return;
    }

    let usage_clamped = usage.clamp(0.0, 100.0);
    let theme = &app.theme;
    let label_style = Style::default()
        .fg(theme.meter_label)
        .add_modifier(Modifier::BOLD);

    match mode {
        MeterMode::Text => {
            // Text mode: just show "N: XX.X%"
            slots.line(
                area,
                [
                    Span::styled(pooled_fmt(format_args!("{:>2}", cpu_idx)), label_style),
                    Span::styled(": ", Style::default().fg(theme.text)),
                    Span::styled(
                        tenths_text(usage_clamped, 5, "%"),
                        Style::default()
                            .fg(theme.cpu_color(usage_clamped))
                            .add_modifier(Modifier::BOLD),
                    ),
                ],
            );
        }
        MeterMode::Graph => {
            // Graph mode: sparkline using history
            let history = app.cpu_history.get(cpu_idx);
            let graph_width =
                (area.width.saturating_sub(10) as usize).min(max_bar_width(area.width as usize)); // label + percent

            let graph: Span<'static> = match history {
                Some(hist) => Span::raw(sparkline_text(hist, graph_width)),
                None => Span::raw(bar_empty(graph_width)),
            };

            slots.line(
                area,
                [
                    Span::styled(pooled_fmt(format_args!("{:>2}[", cpu_idx)), label_style),
                    graph.style(
                        Style::default()
                            .fg(theme.cpu_color(usage_clamped))
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        tenths_text(usage_clamped, 5, "%]"),
                        Style::default().fg(theme.text),
                    ),
                ],
            );
        }
        MeterMode::Bar | MeterMode::Hidden => {
            // Bar mode (default): multi-segment bar with user/system breakdown (htop style)
            // htop uses: nice(blue) + user(green) + system(red) + iowait(gray)
            let bar_width =
                (area.width.saturating_sub(11) as usize).min(max_bar_width(area.width as usize));
            let label = Span::styled(pooled_fmt(format_args!("{:>2}[", cpu_idx)), label_style);
            let percent = Span::styled(
                tenths_text(usage_clamped, 5, "%]"),
                Style::default().fg(theme.text),
            );

            let breakdown = app.system_metrics.cpu.core_breakdown.get(cpu_idx).copied();

            if let Some(bd) = breakdown {
                // Calculate widths for each segment
                let user_pct = bd.user.clamp(0.0, 100.0);
                let system_pct = bd.system.clamp(0.0, 100.0);
                let idle_pct = bd.idle.clamp(0.0, 100.0);

                // htop draws in order: nice, normal(user), system, iowait, irq, softirq, steal, guest
                // We have user and system, with remaining being "other" or idle
                let user_width = ((user_pct * bar_width as f32 / 100.0) as usize).min(bar_width);
                let system_width = ((system_pct * bar_width as f32 / 100.0) as usize)
                    .min(bar_width.saturating_sub(user_width));
                // iowait/other shows as gray - estimated from non-idle, non-user, non-system
                let other_pct = (100.0 - user_pct - system_pct - idle_pct).max(0.0);
                let other_width = ((other_pct * bar_width as f32 / 100.0) as usize)
                    .min(bar_width.saturating_sub(user_width + system_width));
                let empty_width = bar_width.saturating_sub(user_width + system_width + other_width);

                slots.line(
                    area,
                    [
                        label,
                        // User time - green (htop: CPU_NORMAL)
                        Span::styled(bar_fill(user_width), Style::default().fg(theme.cpu_normal)),
                        // System/kernel time - red (htop: CPU_SYSTEM)
                        Span::styled(
                            bar_fill(system_width),
                            Style::default().fg(theme.cpu_system),
                        ),
                        // IO wait/other - gray (htop: CPU_IOWAIT)
                        Span::styled(bar_fill(other_width), Style::default().fg(theme.cpu_iowait)),
                        // Empty space
                        Span::styled(
                            bar_empty(empty_width),
                            Style::default().fg(theme.meter_shadow),
                        ),
                        percent,
                    ],
                );
            } else {
                // Fallback: single color bar based on usage threshold
                let bar_color = theme.cpu_color(usage_clamped);
                let filled = ((usage_clamped as usize) * bar_width / 100).min(bar_width);
                let empty = bar_width - filled;

                slots.line(
                    area,
                    [
                        label,
                        Span::styled(bar_fill(filled), Style::default().fg(bar_color)),
                        Span::styled(bar_empty(empty), Style::default().fg(theme.meter_shadow)),
                        percent,
                    ],
                );
            }
        }
    }
}

/// Render a sparkline graph from history data - htop style
/// Each character encodes TWO consecutive values (left and right halves)
/// This doubles the effective horizontal resolution
fn sparkline_text(history: &VecDeque<f32>, width: usize) -> String {
    let mut result = pooled_string();
    if history.is_empty() || width == 0 {
        result.push_str(bar_empty(width));
        return result;
    }

    // We need width*2 samples since each char shows 2 values
    let samples_needed = width * 2;
    let available_samples = history.len();
    let start = available_samples.saturating_sub(samples_needed);

    // Calculate how many graph chars we can generate and how many spaces we need
    let graph_chars = (available_samples - start).div_ceil(2);
    let graph_chars = graph_chars.min(width);
    let padding_chars = width.saturating_sub(graph_chars);

    result.reserve(width * 3); // UTF-8 braille is 3 bytes

    // Pre-add padding spaces (O(n) instead of O(n²) from repeated insert(0))
    for _ in 0..padding_chars {
        result.push(' ');
    }

    // Process samples in pairs using index-based access
    let mut i = start;
    let mut char_count = 0;
    while i < available_samples && char_count < graph_chars {
        // Left value (older)
        let v1 = history[i];
        // Right value (newer) - use same as left if at end
        let v2 = if i + 1 < available_samples {
            history[i + 1]
        } else {
            v1
        };

        // Map 0-100% to 0-4 (5 levels for braille dots)
        let left = ((v1 / 100.0 * 4.0).round() as usize).min(4);
        let right = ((v2 / 100.0 * 4.0).round() as usize).min(4);

        // Index into 5x5 braille grid
        let idx = left * 5 + right;
        result.push_str(GRAPH_DOTS_UTF8[idx]);
        char_count += 1;
        i += 2;
    }

    result
}

fn draw_memory_bar(slots: &mut Slots, app: &App, area: Rect) {
    let mode = app.config.memory_meter_mode;

    if mode == MeterMode::Hidden {
        return;
    }

    let mem = &app.system_metrics.memory;
    let usage = mem.used_percent.clamp(0.0, 100.0);
    let theme = &app.theme;
    let label_style = Style::default()
        .fg(theme.meter_label)
        .add_modifier(Modifier::BOLD);
    // htop shows "used + shared + compressed" in the info text (line 53-57 of MemoryMeter.c)
    let total_used = mem.used + mem.shared + mem.buffers;
    let mut mem_info = used_total_text(total_used, mem.total);
    let mem_info_len = mem_info.len();

    match mode {
        MeterMode::Text => {
            // Text mode: just show "Mem: XX.X% (used/total)"
            mem_info.insert_str(0, " (");
            mem_info.push(')');
            slots.line(
                area,
                [
                    Span::styled("Mem: ", label_style),
                    Span::styled(
                        tenths_text(usage, 5, "%"),
                        Style::default()
                            .fg(theme.memory_used)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(mem_info, Style::default().fg(theme.text)),
                ],
            );
        }
        MeterMode::Graph => {
            // Graph mode: sparkline using history
            let graph_width = (area.width.saturating_sub(mem_info_len as u16 + 6) as usize)
                .min(max_bar_width(area.width as usize));
            mem_info.push(']');

            slots.line(
                area,
                [
                    Span::styled("Mem[", label_style),
                    Span::styled(
                        sparkline_text(&app.mem_history, graph_width),
                        Style::default()
                            .fg(theme.memory_used)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(mem_info, Style::default().fg(theme.text)),
                ],
            );
        }
        MeterMode::Bar | MeterMode::Hidden => {
            // Bar mode (default): multi-segment bar matching htop exactly
            // htop order: used (green) + shared (magenta) + buffers (blue) + cache (yellow)
            // See htop MemoryMeter.c: MemoryMeter_attributes[]
            let info_len = mem_info_len + 1;
            let bar_width = (area.width.saturating_sub(4 + info_len as u16) as usize)
                .min(max_bar_width(area.width as usize));

            // Calculate segment percentages (htop style)
            let total_f = mem.total as f32;
            let used_pct = if total_f > 0.0 {
                (mem.used as f32 / total_f * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            let shared_pct = if total_f > 0.0 {
                (mem.shared as f32 / total_f * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            let buffers_pct = if total_f > 0.0 {
                (mem.buffers as f32 / total_f * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            let cached_pct = if total_f > 0.0 {
                (mem.cached as f32 / total_f * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };

            // Calculate widths ensuring they don't exceed bar_width
            let used_width = ((used_pct * bar_width as f32 / 100.0) as usize).min(bar_width);
            let shared_width = ((shared_pct * bar_width as f32 / 100.0) as usize)
                .min(bar_width.saturating_sub(used_width));
            let buffers_width = ((buffers_pct * bar_width as f32 / 100.0) as usize)
                .min(bar_width.saturating_sub(used_width + shared_width));
            let cached_width = ((cached_pct * bar_width as f32 / 100.0) as usize)
                .min(bar_width.saturating_sub(used_width + shared_width + buffers_width));
            let empty_width =
                bar_width.saturating_sub(used_width + shared_width + buffers_width + cached_width);
            mem_info.push(']');

            slots.line(
                area,
                [
                    Span::styled("Mem[", label_style),
                    // Used memory - green (htop: MEMORY_USED)
                    Span::styled(bar_fill(used_width), Style::default().fg(theme.memory_used)),
                    // Shared memory - magenta (htop: MEMORY_SHARED)
                    Span::styled(
                        bar_fill(shared_width),
                        Style::default().fg(theme.memory_shared),
                    ),
                    // Buffer cache - blue bold (htop: MEMORY_BUFFERS)
                    Span::styled(
                        bar_fill(buffers_width),
                        Style::default()
                            .fg(theme.memory_buffers)
                            .add_modifier(Modifier::BOLD),
                    ),
                    // Page cache/standby - yellow (htop: MEMORY_CACHE)
                    Span::styled(
                        bar_fill(cached_width),
                        Style::default().fg(theme.memory_cache),
                    ),
                    // Empty/free space
                    Span::styled(
                        bar_empty(empty_width),
                        Style::default().fg(theme.meter_shadow),
                    ),
                    Span::styled(mem_info, Style::default().fg(theme.text)),
                ],
            );
        }
    }
}

fn draw_swap_bar(slots: &mut Slots, app: &App, area: Rect) {
    let mode = app.config.memory_meter_mode;

    if mode == MeterMode::Hidden {
        return;
    }

    let mem = &app.system_metrics.memory;
    let usage = mem.swap_percent.clamp(0.0, 100.0);
    let theme = &app.theme;
    let label_style = Style::default()
        .fg(theme.meter_label)
        .add_modifier(Modifier::BOLD);

    // htop format: "Swp[||||...    X.XXG/X.XXG]"
    let mut swap_info = used_total_text(mem.swap_used, mem.swap_total);
    let swap_info_len = swap_info.len();

    match mode {
        MeterMode::Text => {
            // Text mode: just show "Swp: XX.X% (used/total)"
            swap_info.insert_str(0, " (");
            swap_info.push(')');
            slots.line(
                area,
                [
                    Span::styled("Swp: ", label_style),
                    Span::styled(
                        tenths_text(usage, 5, "%"),
                        Style::default().fg(theme.swap).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(swap_info, Style::default().fg(theme.text)),
                ],
            );
        }
        MeterMode::Graph => {
            // Graph mode: sparkline using history
            let graph_width = (area.width.saturating_sub(swap_info_len as u16 + 6) as usize)
                .min(max_bar_width(area.width as usize));
            swap_info.push(']');

            slots.line(
                area,
                [
                    Span::styled("Swp[", label_style),
                    Span::styled(
                        sparkline_text(&app.swap_history, graph_width),
                        Style::default().fg(theme.swap).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(swap_info, Style::default().fg(theme.text)),
                ],
            );
        }
        MeterMode::Bar | MeterMode::Hidden => {
            // Bar mode (default)
            let info_len = swap_info_len + 1; // +1 for the closing bracket
            let bar_width = (area.width.saturating_sub(4 + info_len as u16) as usize)
                .min(max_bar_width(area.width as usize)); // 4 for "Swp["
            let filled = ((usage as usize) * bar_width / 100).min(bar_width);
            let empty = bar_width - filled;

            // Use theme color for swap bar (htop uses red for swap)
            let bar_color = theme.swap;
            swap_info.push(']');

            slots.line(
                area,
                [
                    Span::styled("Swp[", label_style),
                    Span::styled(bar_fill(filled), Style::default().fg(bar_color)),
                    Span::styled(bar_empty(empty), Style::default().fg(theme.meter_shadow)),
                    Span::styled(swap_info, Style::default().fg(theme.text)),
                ],
            );
        }
    }
}

/// Info text of a GPU/NPU meter: memory first, then the utilization % at the
/// right edge so it lines up with the CPU meters ("Y.YG/Z.ZG X.X%", or
/// "Y.YG X.X%" without a total).
fn adapter_info_text(mem_used: u64, mem_total: u64, usage: f32) -> String {
    let mut info = if mem_total > 0 {
        used_total_text(mem_used, mem_total)
    } else {
        bytes_text("", mem_used, "")
    };
    info.push(' ');
    push_tenths(&mut info, usage, 0);
    info.push('%');
    info
}

/// GPU and NPU meters: `label` is "GPU" or "NPU".
#[allow(clippy::too_many_arguments)] // one meter's label, readings and history
fn draw_adapter_bar(
    slots: &mut Slots,
    app: &App,
    area: Rect,
    mode: MeterMode,
    label: &'static str,
    usage: f32,
    (mem_used, mem_total): (u64, u64),
    history: &VecDeque<f32>,
) {
    let theme = &app.theme;
    let label_style = Style::default()
        .fg(theme.meter_label)
        .add_modifier(Modifier::BOLD);
    // "GPU: " / "GPU[" without formatting: both labels are three ASCII letters.
    let (text_label, bar_label) = match label {
        "NPU" => ("NPU: ", "NPU["),
        _ => ("GPU: ", "GPU["),
    };

    match mode {
        MeterMode::Text => slots.line(
            area,
            [
                Span::styled(text_label, label_style),
                Span::styled(
                    tenths_text(usage, 5, "%"),
                    Style::default()
                        .fg(theme.cpu_color(usage))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    bytes_text(" (", mem_used, ")"),
                    Style::default().fg(theme.text),
                ),
            ],
        ),
        MeterMode::Graph => {
            let mut info = adapter_info_text(mem_used, mem_total, usage);
            let graph_width = (area.width.saturating_sub(info.len() as u16 + 6) as usize)
                .min(max_bar_width(area.width as usize));
            info.push(']');

            slots.line(
                area,
                [
                    Span::styled(bar_label, label_style),
                    Span::styled(
                        sparkline_text(history, graph_width),
                        Style::default()
                            .fg(theme.cpu_color(usage))
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(info, Style::default().fg(theme.text)),
                ],
            );
        }
        MeterMode::Bar | MeterMode::Hidden => {
            let mut info = adapter_info_text(mem_used, mem_total, usage);
            let info_len = info.len() + 1; // +1 for the closing bracket
            let bar_width = (area.width.saturating_sub(4 + info_len as u16) as usize)
                .min(max_bar_width(area.width as usize)); // 4 for "GPU["
            let filled = ((usage as usize) * bar_width / 100).min(bar_width);
            let empty = bar_width - filled;
            info.push(']');

            slots.line(
                area,
                [
                    Span::styled(bar_label, label_style),
                    Span::styled(
                        bar_fill(filled),
                        Style::default().fg(theme.cpu_color(usage)),
                    ),
                    Span::styled(bar_empty(empty), Style::default().fg(theme.meter_shadow)),
                    Span::styled(info, Style::default().fg(theme.text)),
                ],
            );
        }
    }
}

/// GPU utilization meter (Task Manager parity). The bar fill is utilization;
/// the info text prefers dedicated memory (VRAM) when the adapter has any —
/// an aperture commit limit (~half of system RAM) would dwarf the dedicated
/// pool and mislead — falling back to dedicated+shared for iGPUs. Only drawn
/// when a GPU exists (see `gpu_meter_visible`).
fn draw_gpu_bar(slots: &mut Slots, app: &App, area: Rect) {
    let mode = app.config.gpu_meter_mode;

    if mode == MeterMode::Hidden {
        return;
    }

    let Some(gpu) = &app.system_metrics.gpu else {
        return;
    };
    let usage = gpu.utilization.clamp(0.0, 100.0);
    draw_adapter_bar(
        slots,
        app,
        area,
        mode,
        "GPU",
        usage,
        gpu.meter_memory(),
        &app.gpu_history,
    );
}

/// NPU utilization meter (Task Manager parity). The bar fill is utilization;
/// the info text shows NPU memory in use (and total when the driver reports
/// a commit limit). Only drawn when an NPU exists (see `npu_meter_visible`).
fn draw_npu_bar(slots: &mut Slots, app: &App, area: Rect) {
    let mode = app.config.npu_meter_mode;

    if mode == MeterMode::Hidden {
        return;
    }

    let Some(npu) = &app.system_metrics.npu else {
        return;
    };
    let usage = npu.utilization.clamp(0.0, 100.0);
    draw_adapter_bar(
        slots,
        app,
        area,
        mode,
        "NPU",
        usage,
        npu.meter_memory(),
        &app.npu_history,
    );
}

fn draw_tasks_info(slots: &mut Slots, app: &App, area: Rect) {
    let metrics = &app.system_metrics;
    let theme = &app.theme;
    let value_style = Style::default()
        .fg(theme.meter_value)
        .add_modifier(Modifier::BOLD);

    // Windows' native process list does not expose htop-style running/sleeping
    // process state, so do not fabricate a "K running" value.
    // Thread total is already summed once per refresh in SystemMetrics; reuse it
    // instead of re-summing every process on every frame.
    slots.line(
        area,
        [
            Span::styled(
                "Tasks: ",
                Style::default()
                    .fg(theme.meter_label)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                pooled_fmt(format_args!("{}", metrics.tasks_total)),
                value_style,
            ),
            Span::styled(", ", Style::default().fg(theme.text)),
            Span::styled(
                pooled_fmt(format_args!("{}", metrics.threads_total)),
                value_style,
            ),
            Span::styled(" thr", Style::default().fg(theme.text)),
        ],
    );
}

fn draw_uptime_info(slots: &mut Slots, app: &App, area: Rect) {
    let uptime = app.system_metrics.uptime;
    let theme = &app.theme;
    let days = uptime / 86400;
    let hours = (uptime % 86400) / 3600;
    let mins = (uptime % 3600) / 60;
    let secs = uptime % 60;
    let label_style = Style::default()
        .fg(theme.meter_label)
        .add_modifier(Modifier::BOLD);

    // htop format: "Uptime: D day(s), HH:MM:SS"
    let uptime_str = if days > 0 {
        let day_word = if days == 1 { "day" } else { "days" };
        pooled_fmt(format_args!(
            "{} {}, {:02}:{:02}:{:02}",
            days, day_word, hours, mins, secs
        ))
    } else {
        pooled_fmt(format_args!("{:02}:{:02}:{:02}", hours, mins, secs))
    };

    // Calculate overall CPU percentage
    let core_usage = &app.system_metrics.cpu.core_usage;
    let cpu_percent: f32 = if core_usage.is_empty() {
        0.0
    } else {
        core_usage.iter().sum::<f32>() / core_usage.len() as f32
    };

    slots.line(
        area,
        [
            Span::styled("CPU: ", label_style),
            Span::styled(
                tenths_text(cpu_percent, 5, "%"),
                Style::default()
                    .fg(theme.cpu_color(cpu_percent))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("Uptime: ", label_style),
            Span::styled(
                uptime_str,
                Style::default()
                    .fg(theme.uptime)
                    .add_modifier(Modifier::BOLD),
            ),
        ],
    );
}

/// "Net[" / "Dsk[" style rate meter: two labelled `bytes/s` readings.
fn draw_rate_pair(
    slots: &mut Slots,
    app: &App,
    area: Rect,
    label: &'static str,
    first: (&'static str, crate::terminal::Color, u64),
    second: (&'static str, crate::terminal::Color, u64),
) {
    let theme = &app.theme;
    let label_style = Style::default()
        .fg(theme.meter_label)
        .add_modifier(Modifier::BOLD);
    let value_style = Style::default()
        .fg(theme.meter_value)
        .add_modifier(Modifier::BOLD);
    slots.line(
        area,
        [
            Span::styled(label, label_style),
            Span::styled(first.0, Style::default().fg(first.1)),
            Span::styled(bytes_text("", first.2, "/s "), value_style),
            Span::styled(second.0, Style::default().fg(second.1)),
            Span::styled(bytes_text("", second.2, "/s"), value_style),
            Span::styled("]", label_style),
        ],
    );
}

fn draw_network_info(slots: &mut Slots, app: &App, area: Rect) {
    let metrics = &app.system_metrics;
    let theme = &app.theme;

    // htop style: use meter colors for I/O. Green for download, yellow for
    // upload.
    draw_rate_pair(
        slots,
        app,
        area,
        "Net[",
        ("↓", theme.meter_value_ok, metrics.net_rx_rate),
        ("↑", theme.meter_value_warn, metrics.net_tx_rate),
    );
}

fn draw_disk_info(slots: &mut Slots, app: &App, area: Rect) {
    let metrics = &app.system_metrics;
    let theme = &app.theme;

    // htop style: use meter I/O read (green) and write (blue) colors
    draw_rate_pair(
        slots,
        app,
        area,
        "Dsk[",
        ("R:", theme.meter_value_ok, metrics.disk_read_rate),
        ("W:", theme.memory_buffers, metrics.disk_write_rate),
    );
}

fn draw_battery_info(slots: &mut Slots, app: &App, area: Rect) {
    let metrics = &app.system_metrics;
    let theme = &app.theme;
    let label_style = Style::default()
        .fg(theme.meter_label)
        .add_modifier(Modifier::BOLD);

    if let Some(percent) = metrics.battery_percent {
        let status = if metrics.battery_charging { "+" } else { "-" };
        let color = if percent > 50.0 {
            theme.meter_value_ok // Green
        } else if percent > 20.0 {
            theme.meter_value_warn // Yellow
        } else {
            theme.meter_value_error // Red
        };
        let mut value = pooled_string();
        push_round0(&mut value, percent);
        value.push('%');

        slots.line(
            area,
            [
                Span::styled("Bat[", label_style),
                Span::styled(
                    status,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    value,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled("]", label_style),
            ],
        );
    } else {
        // No battery detected, show hostname instead (htop style)
        slots.line(
            area,
            [
                Span::styled("Host: ", label_style),
                Span::styled(
                    pooled_str(&metrics.hostname),
                    Style::default()
                        .fg(theme.hostname)
                        .add_modifier(Modifier::BOLD),
                ),
            ],
        );
    }
}

#[cfg(test)]
mod layout_equivalence_tests {
    use super::{length_row_rect, ratio_column_rect};
    use crate::terminal::{Constraint, Direction, Layout, Rect};

    #[test]
    fn ratio_columns_match_layout_split() {
        for width in 1u16..=300 {
            for cols in 1usize..=4 {
                let inner = Rect::new(3, 7, width, 5);
                let constraints: Vec<Constraint> = (0..cols)
                    .map(|_| Constraint::Ratio(1, cols as u32))
                    .collect();
                let expected = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints(constraints)
                    .split(inner);
                for col_idx in 0..cols {
                    assert_eq!(
                        ratio_column_rect(inner, col_idx, cols),
                        expected[col_idx],
                        "width={width} cols={cols} col={col_idx}"
                    );
                }
            }
        }
    }

    #[test]
    fn length_rows_match_layout_split() {
        for height in 0u16..=20 {
            for n in 1usize..=12 {
                let area = Rect::new(2, 4, 30, height);
                let constraints: Vec<Constraint> = (0..n).map(|_| Constraint::Length(1)).collect();
                let expected = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(constraints)
                    .split(area);
                for row_idx in 0..n {
                    assert_eq!(
                        length_row_rect(area, row_idx, n),
                        expected[row_idx],
                        "height={height} n={n} row={row_idx}"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod filler_plan_tests {
    use super::filler_plan;
    use crate::app::App;
    use crate::config::Config;

    fn app_with_cores(cpus: usize, width: u16, show_header: bool) -> App {
        let mut app = App::new(Config::default());
        app.show_header = show_header;
        app.terminal_width = width;
        app.system_metrics.cpu.core_usage = vec![0.0; cpus];
        app
    }

    #[test]
    fn filler_plan_follows_slot_arithmetic() {
        // Hidden header: nothing collects.
        let app = app_with_cores(8, 120, false);
        assert_eq!(filler_plan(&app), [false, false, false]);

        // Single column: no filler-hosting columns.
        let app = app_with_cores(8, 60, true);
        assert_eq!(filler_plan(&app), [false, false, false]);

        // 2 columns @ 8 CPUs / 4 meter rows: both columns are full, so no
        // filler renders anywhere.
        let app = app_with_cores(8, 120, true);
        assert_eq!(filler_plan(&app), [false, false, false]);

        // 2 columns @ 6 CPUs / 4 meter rows: the right column hosts fillers
        // and has one empty CPU slot → Net only.
        let app = app_with_cores(6, 120, true);
        assert_eq!(filler_plan(&app), [true, false, false]);

        // 2 columns @ 4 CPUs / 4 meter rows: two empty slots → Net then Dsk.
        let app = app_with_cores(4, 120, true);
        assert_eq!(filler_plan(&app), [true, true, false]);
    }
}
