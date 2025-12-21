use sysinfo::System;

/// Memory usage information
#[derive(Default, Clone)]
pub struct MemoryInfo {
    /// Total physical memory in bytes
    pub total: u64,
    /// Used physical memory in bytes
    pub used: u64,
    /// Memory used percentage
    pub used_percent: f32,
    /// Total swap in bytes
    pub swap_total: u64,
    /// Used swap in bytes
    pub swap_used: u64,
    /// Swap used percentage
    pub swap_percent: f32,
}

impl MemoryInfo {
    pub fn from_sysinfo(sys: &System) -> Self {
        let total = sys.total_memory();
        let used = total.saturating_sub(sys.available_memory());

        let swap_total = sys.total_swap();
        let swap_used = sys.used_swap();

        Self {
            total,
            used,
            used_percent: if total > 0 { used as f32 / total as f32 * 100.0 } else { 0.0 },
            swap_total,
            swap_used,
            swap_percent: if swap_total > 0 { swap_used as f32 / swap_total as f32 * 100.0 } else { 0.0 },
        }
    }
}

/// Format bytes into human-readable string
pub fn format_bytes(bytes: u64) -> String {
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
