//! Deterministic visual smoke test for the htop-win TUI.
//!
//! The test renders the real app UI into the in-memory terminal buffer and
//! compares it with raw snapshot lines. It intentionally avoids live system data
//! so failures reflect rendering changes, not the machine running the test.

use std::time::Duration;

use htop_win::app::{App, ScreenTab, SortColumn};
use htop_win::config::Config;
use htop_win::system::{
    CpuInfo, GpuInfo, MemoryInfo, NpuInfo, ProcessArch, ProcessInfo, SystemMetrics,
};
use htop_win::terminal::{Buffer, Frame, Rect};

fn fixture_process(pid: u32, user: &str, name: &str, cpu: f32, mem: f32) -> ProcessInfo {
    ProcessInfo {
        pid,
        parent_pid: 4,
        name: name.into(),
        exe_path: format!(r"C:\Program Files\{name}\{name}.exe").into(),
        command: format!(r"C:\Program Files\{name}\{name}.exe --fixture").into(),
        user: user.into(),
        status: if cpu > 20.0 { 'R' } else { 'S' },
        cpu_percent: cpu,
        mem_percent: mem,
        virtual_mem: 256 * 1024 * 1024,
        resident_mem: 64 * 1024 * 1024,
        shared_mem: 16 * 1024 * 1024,
        priority: 20,
        cpu_time: Duration::from_secs(75),
        tree_depth: 0,
        tree_prefix: String::new(),
        has_children: false,
        is_collapsed: false,
        thread_count: 8,
        start_time: 1_700_000_000,
        create_time_100ns: 133_444_736_000_000_000 + pid as u64,
        handle_count: 128,
        io_read_bytes: 1024,
        io_write_bytes: 2048,
        io_read_rate: 512,
        io_write_rate: 256,
        gpu_percent: 0.0,
        gpu_memory: 0,
        npu_percent: 0.0,
        npu_memory: 0,
        name_lower: name.to_lowercase().into(),
        command_lower: name.to_lowercase().into(),
        user_lower: user.to_lowercase().into(),
        matches_search: false,
        efficiency_mode: false,
        is_elevated: false,
        arch: ProcessArch::Native,
        exe_updated: false,
        exe_deleted: false,
    }
}

fn fixture_app() -> App {
    let mut config = Config::default();
    config.visible_columns = vec![
        "PID".to_string(),
        "USER".to_string(),
        "CPU%".to_string(),
        "MEM%".to_string(),
        "TIME+".to_string(),
        "Command".to_string(),
    ];
    config.highlight_large_numbers = false;

    let mut app = App::new(config.clone());
    app.show_header = false;
    app.screen_tabs = vec![ScreenTab {
        name: "Main".to_string(),
        columns: config.visible_columns.clone(),
        sort_column: SortColumn::Pid,
        sort_ascending: true,
    }];
    app.active_tab = 0;
    app.sort_column = SortColumn::Pid;
    app.sort_ascending = true;
    app.update_visible_columns_cache();

    let mut metrics = SystemMetrics::default();
    metrics.cpu = CpuInfo {
        core_usage: vec![12.5, 25.0],
        core_breakdown: Vec::new(),
    };
    metrics.memory = MemoryInfo {
        total: 8 * 1024 * 1024 * 1024,
        used: 3 * 1024 * 1024 * 1024,
        shared: 512 * 1024 * 1024,
        buffers: 0,
        cached: 1024 * 1024 * 1024,
        used_percent: 37.5,
        swap_total: 2 * 1024 * 1024 * 1024,
        swap_used: 0,
        swap_percent: 0.0,
    };
    metrics.tasks_total = 3;
    metrics.threads_total = 24;
    app.system_metrics = metrics;

    app.processes = vec![
        fixture_process(100, "SYSTEM", "SystemIdle", 0.0, 0.1),
        fixture_process(200, "alice", "Shell", 12.5, 1.5),
        fixture_process(300, "builder", "RustCompiler", 42.0, 4.0),
    ];
    app.update_displayed_processes();
    app
}

fn render_fixture_text() -> String {
    let mut app = fixture_app();
    let area = Rect::new(0, 0, 120, 10);
    let mut buffer = Buffer::empty(area);
    let mut frame = Frame::new(&mut buffer);

    htop_win::ui::draw(&mut frame, &mut app);

    (0..area.height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..area.width {
                if let Some(cell) = buffer.get(x, y) {
                    line.push_str(&cell.symbol);
                }
            }
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_cells(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn visual_snapshot_renders_real_app() {
    let actual = render_fixture_text();
    let snapshot = include_str!("snapshots/htop_reference.txt");
    let actual_lines = actual.lines().map(normalize_cells).collect::<Vec<_>>();

    assert!(
        !snapshot.contains("LINE 01:"),
        "visual snapshot must be raw rendered text, not annotated prose"
    );

    for expected in snapshot.lines().filter(|line| !line.trim().is_empty()) {
        let expected = normalize_cells(expected);
        assert!(
            actual_lines.iter().any(|line| line.contains(&expected)),
            "rendered UI did not contain snapshot line `{expected}`\n\nactual:\n{actual}"
        );
    }
}

#[test]
fn reset_hardware_columns_render_elevated_system_with_wide_indicator() {
    let mut config = Config::default();
    config.visible_columns = vec![
        "PID".to_string(),
        "USER".to_string(),
        "PRI".to_string(),
        "CLASS".to_string(),
        "THR".to_string(),
        "VIRT".to_string(),
        "RES".to_string(),
        "SHR".to_string(),
        "S".to_string(),
        "CPU%".to_string(),
        "MEM%".to_string(),
        "TIME+".to_string(),
        "Command".to_string(),
    ];
    config.highlight_large_numbers = false;

    let mut app = App::new(config.clone());
    app.show_header = false;
    app.screen_tabs = vec![ScreenTab {
        name: "Main".to_string(),
        columns: config.visible_columns.clone(),
        sort_column: SortColumn::Pid,
        sort_ascending: true,
    }];
    app.active_tab = 0;
    app.sort_column = SortColumn::Pid;
    app.sort_ascending = true;

    let mut metrics = SystemMetrics::default();
    metrics.gpu = Some(GpuInfo {
        name: "Fixture GPU".to_string(),
        utilization: 10.0,
        mem_used: 512 * 1024 * 1024,
        mem_total: 4 * 1024 * 1024 * 1024,
        dedicated_used: 0,
        dedicated_total: 0,
        shared_used: 0,
    });
    metrics.npu = Some(NpuInfo {
        name: "Fixture NPU".to_string(),
        utilization: 0.0,
        mem_used: 0,
        mem_total: 1024 * 1024 * 1024,
        dedicated_used: 0,
        dedicated_total: 0,
        shared_used: 0,
    });
    app.system_metrics = metrics;

    let mut system = fixture_process(4, "SYSTEM", "System", 0.1, 0.3);
    system.command = "System".into();
    system.exe_path = "".into();
    system.is_elevated = true;
    system.thread_count = 316;
    app.processes = vec![
        fixture_process(1, "SYSTEM", "Idle", 0.0, 0.0),
        system,
        fixture_process(200, "alice", "Shell", 12.5, 1.5),
    ];
    app.selected_index = 0;
    app.update_displayed_processes();

    app.config.reset_to_defaults();
    app.reset_screen_tabs();
    app.update_visible_columns_cache();

    let area = Rect::new(0, 0, 120, 10);
    let mut buffer = Buffer::empty(area);
    let mut frame = Frame::new(&mut buffer);
    htop_win::ui::draw(&mut frame, &mut app);

    for y in 0..area.height {
        for x in 0..area.width.saturating_sub(3) {
            let cell = buffer.get(x, y).unwrap();
            if cell.symbol == "🛡️" {
                assert!(buffer.get(x + 1, y).unwrap().is_continuation);
                assert_eq!(buffer.get(x + 2, y).unwrap().symbol, " ");
                assert_eq!(buffer.get(x + 3, y).unwrap().symbol, "S");
                return;
            }
        }
    }

    panic!("rendered reset view did not contain elevated System command row");
}
