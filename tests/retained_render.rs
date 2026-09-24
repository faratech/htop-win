//! Differential gate for the retained compositor.
//!
//! Every frame composed incrementally (unchanged rows skipped, only changed
//! rows repainted) must be cell-for-cell identical to a from-scratch render of
//! the same app state. Drives the real UI through a seeded random walk over
//! everything that shapes the screen, plus scripted edge cases, and checks
//! the dirty-row budgets that make the retained frame worth having.

use std::time::{Duration, Instant};

use htop_win::app::{App, DialogState, ScreenTab};
use htop_win::config::{Config, MeterMode};
use htop_win::data::SystemSnapshot;
use htop_win::system::{
    CpuInfo, GpuInfo, MemoryInfo, ProcessArch, ProcessEnrichmentRequirements, ProcessInfo,
    SystemMetrics,
};
use htop_win::terminal::{Buffer, Color, Compositor, Frame, Rect};
use htop_win::ui::colors::ColorScheme;

/// Fixed render clock: START and the new-process highlight depend on it.
const NOW: u64 = 1_800_000_000;

/// Deterministic xorshift.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn process(i: u32, rng: &mut Rng) -> ProcessInfo {
    let names = [
        "chrome.exe",
        "svchost.exe",
        "Code.exe",
        "进程.exe",
        "rust-analyzer.exe",
    ];
    let users = ["alice", "SYSTEM", "用户名", "LOCAL SERVICE", "bob"];
    let name = names[(i as usize) % names.len()];
    let user = users[(i as usize * 7) % users.len()];
    let command = format!(r"C:\Program Files\App{}\{}", i % 4, name);
    ProcessInfo {
        pid: 0x7FF0_0000 + i * 4, // never a live PID: enrichment fails fast
        parent_pid: if i.is_multiple_of(5) {
            4
        } else {
            0x7FF0_0000 + (i / 5) * 20
        },
        name: name.into(),
        exe_path: command.clone().into(),
        command: command.clone().into(),
        user: user.into(),
        status: if i.is_multiple_of(3) { b'R' } else { b'?' },
        cpu_percent: rng.below(1000) as f32 / 10.0,
        mem_percent: rng.below(300) as f32 / 10.0,
        virtual_mem: rng.below(1 << 42),
        resident_mem: rng.below(1 << 34),
        shared_mem: rng.below(1 << 28),
        priority: [4, 6, 8, 10, 13, 24][(i % 6) as usize],
        cpu_time: rng.below(10_000_000 * 3600 * 30),
        thread_count: 1 + rng.below(80) as u32,
        start_time: (NOW - rng.below(200_000)) as u32,
        create_time_100ns: 133_000_000_000_000_000 + u64::from(i),
        handle_count: rng.below(5000) as u32,
        io_read_bytes: rng.below(1 << 40),
        io_write_bytes: rng.below(1 << 30),
        io_read_rate: rng.below(1 << 24),
        io_write_rate: rng.below(1 << 20),
        gpu_percent: rng.below(1000) as f32 / 10.0,
        gpu_memory: rng.below(1 << 32),
        npu_percent: 0.0,
        npu_memory: 0,
        name_lower: name.to_lowercase().into(),
        command_lower: command.to_lowercase().into(),
        user_lower: user.to_lowercase().into(),
        efficiency_mode: i.is_multiple_of(4),
        is_elevated: i % 3 == 1,
        arch: [ProcessArch::Native, ProcessArch::X86, ProcessArch::X64][(i % 3) as usize],
        exe_updated: i.is_multiple_of(11),
        exe_deleted: i.is_multiple_of(13),
    }
}

fn metrics(rng: &mut Rng, cores: usize) -> SystemMetrics {
    let mut metrics = SystemMetrics::default();
    metrics.cpu = CpuInfo {
        core_usage: (0..cores).map(|_| rng.below(1000) as f32 / 10.0).collect(),
        core_breakdown: Vec::new(),
    };
    metrics.memory = MemoryInfo {
        total: 32 << 30,
        used: rng.below(30 << 30),
        shared: 1 << 29,
        buffers: 0,
        cached: 4 << 30,
        used_percent: rng.below(1000) as f32 / 10.0,
        swap_total: 8 << 30,
        swap_used: rng.below(8 << 30),
        swap_percent: rng.below(1000) as f32 / 10.0,
    };
    metrics.uptime = rng.below(10_000_000);
    metrics.tasks_total = 300 + rng.below(200) as usize;
    metrics.threads_total = 5000 + rng.below(3000) as usize;
    metrics.net_rx_rate = rng.below(1 << 24);
    metrics.net_tx_rate = rng.below(1 << 20);
    metrics.disk_read_rate = rng.below(1 << 26);
    metrics.disk_write_rate = rng.below(1 << 22);
    metrics.hostname = "retained-test".to_string();
    metrics
}

fn gpu(rng: &mut Rng, name: &str) -> GpuInfo {
    GpuInfo {
        name: name.to_string(),
        utilization: rng.below(1000) as f32 / 10.0,
        mem_used: rng.below(8 << 30),
        mem_total: 8 << 30,
        dedicated_used: rng.below(8 << 30),
        dedicated_total: 8 << 30,
        shared_used: rng.below(1 << 30),
    }
}

/// Deliver processes the way the collector does, with every metadata
/// dependency satisfied so filters and dependent sorts apply immediately.
fn snapshot(metrics: SystemMetrics, processes: Vec<ProcessInfo>) -> SystemSnapshot {
    SystemSnapshot {
        metrics,
        processes,
        refresh_duration: Duration::ZERO,
        enrichment: ProcessEnrichmentRequirements::visible(true),
        published_at: Instant::now(),
    }
}

const COLUMN_SETS: &[&[&str]] = &[
    &[
        "PID", "USER", "PRI", "CLASS", "THR", "VIRT", "RES", "SHR", "S", "CPU%", "MEM%", "TIME+",
        "Command",
    ],
    &["PID", "USER", "CPU%", "Command"],
    &["PID", "ELEV", "ARCH", "ECO", "START", "S", "Command"],
    &[
        "PID", "PPID", "IO_RATE", "IO_R/s", "IO_W/s", "IO_RD", "IO_WR", "HNDL", "GPU%", "GPU-MEM",
        "Command",
    ],
    &["Command", "PID"],
];

struct Harness {
    app: App,
    compositor: Compositor,
    area: Rect,
    frames: usize,
}

impl Harness {
    fn new(area: Rect) -> Self {
        let mut app = App::new(Config::default());
        app.clock_override = Some(NOW);
        Self {
            app,
            compositor: Compositor::new(area),
            area,
            frames: 0,
        }
    }

    /// Compose incrementally, render the same state from scratch, and require
    /// identical cells. Leaves the compositor's dirty flags for inspection
    /// until the next call.
    fn frame(&mut self, label: &str) {
        self.compositor.acknowledge();
        let Harness {
            app,
            compositor,
            area,
            ..
        } = self;
        compositor.draw(*area, |f| htop_win::ui::draw(f, app));
        let mut reference = Buffer::empty(*area);
        htop_win::ui::draw(&mut Frame::new(&mut reference), app);
        self.frames += 1;

        let retained = self.compositor.buffer();
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if retained.get(x, y) != reference.get(x, y) {
                    panic!(
                        "frame {} ({label}): cell ({x},{y}) differs\n retained: {:?}\nreference: {:?}\n retained row: {}\nreference row: {}",
                        self.frames,
                        retained.get(x, y),
                        reference.get(x, y),
                        row_text(retained, y),
                        row_text(&reference, y),
                    );
                }
            }
        }
    }

    fn apply(&mut self, rng: &mut Rng, cores: usize, count: u32) {
        let processes = (0..count).map(|i| process(i, rng)).collect();
        let mut metrics = metrics(rng, cores);
        metrics.gpu = self
            .app
            .system_metrics
            .gpu
            .clone()
            .map(|_| gpu(rng, "GPU A"));
        metrics.npu = self.app.system_metrics.npu.clone().map(|_| gpu(rng, "NPU"));
        metrics.battery_percent = self
            .app
            .system_metrics
            .battery_percent
            .map(|_| rng.below(101) as f32);
        drop(self.app.apply_snapshot(snapshot(metrics, processes)));
    }
}

fn row_text(buffer: &Buffer, y: u16) -> String {
    (buffer.area.x..buffer.area.right())
        .filter_map(|x| buffer.get(x, y))
        .filter(|cell| !cell.is_continuation)
        .map(|cell| cell.symbol.as_str().to_string())
        .collect()
}

fn set_columns(app: &mut App, columns: &[&str]) {
    let columns: Vec<String> = columns.iter().map(|c| c.to_string()).collect();
    app.screen_tabs[0].columns = columns;
    app.active_tab = 0;
    app.update_visible_columns_cache();
    app.needs_process_update = true;
}

fn cycle_meter(mode: MeterMode) -> MeterMode {
    match mode {
        MeterMode::Bar => MeterMode::Text,
        MeterMode::Text => MeterMode::Graph,
        MeterMode::Graph => MeterMode::Hidden,
        MeterMode::Hidden => MeterMode::Bar,
    }
}

fn open_dialog(app: &mut App, which: u64) {
    match which % 15 {
        0 => {
            app.handle_function_key(1);
        }
        1 => app.start_search(),
        2 => app.start_filter(),
        3 => {
            app.handle_function_key(6);
        }
        4 => app.enter_kill_mode(),
        5 => app.enter_priority_mode(1),
        6 => {
            app.handle_function_key(2);
        }
        7 => app.enter_process_info_mode(),
        8 => {
            app.dialog = DialogState::UserSelect {
                index: 1,
                users: vec!["alice".into(), "用户名".into(), "SYSTEM".into()],
            }
        }
        9 => app.enter_environment_mode(),
        10 => app.dialog = DialogState::ColorScheme { index: 2 },
        11 => {
            app.dialog = DialogState::GpuSelect {
                index: 1,
                names: vec!["GPU A".into(), "GPU 🛡️ B".into()],
            }
        }
        12 => app.enter_command_wrap_mode(),
        13 => app.enter_column_config_mode(),
        _ => app.enter_affinity_mode(),
    }
}

#[test]
fn retained_frames_match_full_renders_through_a_random_walk() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut h = Harness::new(Rect::new(0, 0, 120, 30));
    let mut cores = 12;
    let mut count = 60;
    h.apply(&mut rng, cores, count);
    h.frame("initial");

    for step in 0..2000 {
        let label = match rng.below(26) {
            0..=4 => {
                h.apply(&mut rng, cores, count);
                "snapshot"
            }
            5 => {
                h.app.select_down();
                "down"
            }
            6 => {
                h.app.select_up();
                "up"
            }
            7 => {
                h.app.page_down();
                "page down"
            }
            8 => {
                if rng.below(2) == 0 {
                    h.app.select_last();
                } else {
                    h.app.select_first();
                }
                "jump"
            }
            9 => {
                h.app.toggle_tag();
                "tag"
            }
            10 => {
                let search = ["", "chrome", "进程", "zzz"][rng.below(4) as usize];
                h.app.search_string = search.to_string();
                h.app.search_string_lower = search.to_lowercase();
                h.app.needs_process_update = true;
                "search"
            }
            11 => {
                let filter = ["", "svc", "alice", "nomatch", "进"][rng.below(5) as usize];
                h.app.filter_string = filter.to_string();
                h.app.filter_string_lower = filter.to_lowercase();
                h.app.needs_process_update = true;
                "filter"
            }
            12 => {
                h.app.tree_view = !h.app.tree_view;
                h.app.needs_process_update = true;
                "tree"
            }
            13 => {
                set_columns(
                    &mut h.app,
                    COLUMN_SETS[rng.below(COLUMN_SETS.len() as u64) as usize],
                );
                "columns"
            }
            14 => {
                if rng.below(2) == 0 {
                    let schemes = ColorScheme::all();
                    h.app.config.color_scheme = schemes[rng.below(schemes.len() as u64) as usize];
                    h.app.update_theme();
                    "theme"
                } else {
                    // Background alone: rows whose spans carry no background
                    // (header meters) must still repaint.
                    let backgrounds =
                        [Color::Reset, Color::Black, Color::Blue, Color::Rgb(9, 9, 9)];
                    h.app.theme.background = backgrounds[rng.below(4) as usize];
                    "background"
                }
            }
            15 => {
                h.app.show_header = !h.app.show_header;
                "header"
            }
            16 => {
                if h.app.screen_tabs.len() > 1 {
                    h.app.screen_tabs.truncate(1);
                    h.app.active_tab = 0;
                } else {
                    h.app.screen_tabs.push(ScreenTab::default_io());
                }
                "tabs"
            }
            17 => {
                let m = &mut h.app.system_metrics;
                match rng.below(3) {
                    0 => m.gpu = m.gpu.take().xor(Some(gpu(&mut rng, "GPU A"))),
                    1 => m.npu = m.npu.take().xor(Some(gpu(&mut rng, "NPU"))),
                    _ => m.battery_percent = m.battery_percent.xor(Some(55.0)),
                }
                "adapters"
            }
            18 => {
                let config = &mut h.app.config;
                match rng.below(4) {
                    0 => config.cpu_meter_mode = cycle_meter(config.cpu_meter_mode),
                    1 => config.memory_meter_mode = cycle_meter(config.memory_meter_mode),
                    2 => config.gpu_meter_mode = cycle_meter(config.gpu_meter_mode),
                    _ => config.highlight_large_numbers = !config.highlight_large_numbers,
                }
                "meters"
            }
            19 => {
                cores = [1, 4, 12, 64][rng.below(4) as usize];
                count = [0, 3, 60, 200][rng.below(4) as usize];
                h.apply(&mut rng, cores, count);
                "population"
            }
            20 => {
                let widths = [20, 45, 80, 120, 151, 230];
                let heights = [0, 1, 2, 5, 10, 24, 30, 50];
                h.area = Rect::new(
                    0,
                    0,
                    widths[rng.below(widths.len() as u64) as usize],
                    heights[rng.below(heights.len() as u64) as usize],
                );
                "resize"
            }
            21 | 22 => {
                if matches!(h.app.dialog, DialogState::None) {
                    open_dialog(&mut h.app, rng.next());
                } else {
                    h.app.dialog = DialogState::None;
                }
                "dialog"
            }
            23 => {
                h.app.last_error = if h.app.last_error.is_some() {
                    None
                } else {
                    Some((
                        "simulated failure with 🛡️ and 用户名".into(),
                        Instant::now(),
                    ))
                };
                "error"
            }
            24 => {
                h.app.status_message = Some(if rng.below(2) == 0 {
                    ("Settings saved".to_string(), Instant::now())
                } else {
                    // Long expired: hidden.
                    let at = Instant::now().checked_sub(Duration::from_secs(60));
                    ("expired".to_string(), at.unwrap_or_else(Instant::now))
                });
                "status"
            }
            _ => {
                h.app.paused = !h.app.paused;
                "pause"
            }
        };
        if h.app.needs_process_update {
            h.app.update_displayed_processes();
            h.app.needs_process_update = false;
        }
        h.frame(&format!("step {step}: {label}"));
    }
}

#[test]
fn dialogs_over_wide_glyph_rows_close_cleanly() {
    let mut rng = Rng(7);
    for width in [80u16, 97, 120, 133] {
        let mut h = Harness::new(Rect::new(0, 0, width, 32));
        set_columns(&mut h.app, COLUMN_SETS[2]);
        h.apply(&mut rng, 8, 40);
        h.frame("base");
        for which in 0..15 {
            open_dialog(&mut h.app, which);
            h.frame(&format!("width {width}: open dialog {which}"));
            h.app.dialog = DialogState::None;
            h.frame(&format!("width {width}: close dialog {which}"));
        }
    }
}

/// A settled frame: compose twice so the second reflects only real changes.
fn settled(area: Rect) -> (Harness, Rng) {
    let mut rng = Rng(99);
    let mut h = Harness::new(area);
    h.apply(&mut rng, 8, 60);
    h.frame("settle 1");
    h.frame("settle 2");
    (h, rng)
}

#[test]
fn an_unchanged_frame_repaints_nothing() {
    let (mut h, _) = settled(Rect::new(0, 0, 120, 30));
    h.frame("unchanged");
    assert_eq!(h.compositor.buffer().dirty_rows(), 0);
}

#[test]
fn moving_the_selection_within_the_page_repaints_two_rows() {
    let (mut h, _) = settled(Rect::new(0, 0, 120, 30));
    h.app.select_down();
    h.frame("select down");
    assert_eq!(h.compositor.buffer().dirty_rows(), 2);
}

#[test]
fn a_snapshot_leaves_static_rows_untouched() {
    let area = Rect::new(0, 0, 120, 30);
    let (mut h, mut rng) = settled(area);
    let tab_bar_y = h.app.ui_bounds.tab_bar_y;
    let column_header_y = h.app.ui_bounds.column_header_y;
    let footer_y = h.app.ui_bounds.footer_y_start;
    h.apply(&mut rng, 8, 60);
    h.frame("snapshot");

    let buffer = h.compositor.buffer();
    assert!(buffer.dirty_rows() > 0, "the snapshot changed something");
    for (y, what) in [
        (tab_bar_y, "tab bar"),
        (column_header_y, "column header"),
        (footer_y, "function-key row"),
        (footer_y + 1, "status line"),
    ] {
        assert!(!buffer.row_is_dirty(y), "{what} (row {y}) was repainted");
    }
}
