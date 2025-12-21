use sysinfo::System;

/// CPU usage information
#[derive(Default, Clone)]
pub struct CpuInfo {
    /// Per-core CPU usage percentages
    pub core_usage: Vec<f32>,
}

impl CpuInfo {
    pub fn from_sysinfo(sys: &System) -> Self {
        Self {
            core_usage: sys.cpus().iter().map(|c| c.cpu_usage()).collect(),
        }
    }
}
