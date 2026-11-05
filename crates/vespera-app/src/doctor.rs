//! Read-only host evidence, not inference fit detection or GPU telemetry.

use std::fs;
use std::path::Path;

use serde::Serialize;

/// Observations from this process's host. Missing evidence remains unknown.
#[derive(Debug, Serialize)]
pub struct SystemReport {
    pub os: &'static str,
    pub architecture: &'static str,
    pub logical_cpu_threads_available: Option<usize>,
    pub linux_host_ram_total_bytes: Option<u64>,
    pub linux_host_ram_available_bytes: Option<u64>,
    pub cgroup_v2_memory: MemoryLimit,
    pub cgroup_v2_cpu: CpuLimit,
    pub exposed_gpu_device_paths: Vec<String>,
    pub gpu_vram_bytes: Option<u64>,
    pub gpu_telemetry_status: &'static str,
    pub notice: &'static str,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "status", content = "bytes", rename_all = "snake_case")]
pub enum MemoryLimit {
    Unknown,
    Unlimited,
    Limited(u64),
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "status", content = "cores", rename_all = "snake_case")]
pub enum CpuLimit {
    Unknown,
    Unlimited,
    Limited(f64),
}

impl SystemReport {
    pub fn collect() -> Self {
        let meminfo = fs::read_to_string("/proc/meminfo").ok();
        let memory_limit = fs::read_to_string("/sys/fs/cgroup/memory.max")
            .map(|input| parse_memory_limit(&input))
            .unwrap_or(MemoryLimit::Unknown);
        let cpu_limit = fs::read_to_string("/sys/fs/cgroup/cpu.max")
            .map(|input| parse_cpu_limit(&input))
            .unwrap_or(CpuLimit::Unknown);
        Self {
            os: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            logical_cpu_threads_available: std::thread::available_parallelism()
                .ok()
                .map(usize::from),
            linux_host_ram_total_bytes: meminfo
                .as_deref()
                .and_then(|input| meminfo_bytes(input, "MemTotal:")),
            linux_host_ram_available_bytes: meminfo
                .as_deref()
                .and_then(|input| meminfo_bytes(input, "MemAvailable:")),
            cgroup_v2_memory: memory_limit,
            cgroup_v2_cpu: cpu_limit,
            exposed_gpu_device_paths: gpu_device_paths(),
            gpu_vram_bytes: None,
            gpu_telemetry_status: "not implemented; device paths do not establish GPU usability",
            notice: "Host observations are not admission decisions. Linux RAM may describe a host rather than the container. Root cgroup files may not describe this process's nested cgroup. No provider connections or model activations were attempted.",
        }
    }
}

fn meminfo_bytes(input: &str, key: &str) -> Option<u64> {
    input.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        if fields.next()? != key {
            return None;
        }
        let value = fields.next()?.parse::<u64>().ok()?;
        if fields.next()? != "kB" {
            return None;
        }
        value.checked_mul(1024)
    })
}

fn parse_memory_limit(input: &str) -> MemoryLimit {
    match input.trim() {
        "max" => MemoryLimit::Unlimited,
        value => value
            .parse::<u64>()
            .map(MemoryLimit::Limited)
            .unwrap_or(MemoryLimit::Unknown),
    }
}

fn parse_cpu_limit(input: &str) -> CpuLimit {
    let mut values = input.split_whitespace();
    let Some(quota) = values.next() else {
        return CpuLimit::Unknown;
    };
    let Some(period) = values.next().and_then(|value| value.parse::<u64>().ok()) else {
        return CpuLimit::Unknown;
    };
    if period == 0 || values.next().is_some() {
        return CpuLimit::Unknown;
    }
    if quota == "max" {
        return CpuLimit::Unlimited;
    }
    match quota.parse::<u64>() {
        Ok(value) if value > 0 => CpuLimit::Limited(value as f64 / period as f64),
        _ => CpuLimit::Unknown,
    }
}

fn gpu_device_paths() -> Vec<String> {
    let mut devices = Vec::new();
    if let Ok(entries) = fs::read_dir("/dev") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Some(name) = name.to_str()
                && let Some(index) = name.strip_prefix("nvidia")
                && !index.is_empty()
                && index.chars().all(|ch| ch.is_ascii_digit())
            {
                devices.push(entry.path().display().to_string());
            }
        }
    }
    if Path::new("/dev/dri").is_dir()
        && let Ok(entries) = fs::read_dir("/dev/dri")
    {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("renderD") {
                devices.push(entry.path().display().to_string());
            }
        }
    }
    devices.sort();
    devices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_memory_with_checked_units() {
        assert_eq!(
            meminfo_bytes("MemTotal: 1024 kB\n", "MemTotal:"),
            Some(1048576)
        );
        assert_eq!(
            meminfo_bytes("MemAvailable: 0 kB", "MemAvailable:"),
            Some(0)
        );
        assert_eq!(meminfo_bytes("MemTotal: 1024 MB", "MemTotal:"), None);
        assert_eq!(
            meminfo_bytes("MemTotal: 18446744073709551615 kB", "MemTotal:"),
            None
        );
        assert_eq!(meminfo_bytes("bad input", "MemTotal:"), None);
    }

    #[test]
    fn cgroup_limits_distinguish_unknown_and_unlimited() {
        assert_eq!(parse_memory_limit("max\n"), MemoryLimit::Unlimited);
        assert_eq!(parse_memory_limit("4096"), MemoryLimit::Limited(4096));
        assert_eq!(parse_memory_limit("?"), MemoryLimit::Unknown);
        assert_eq!(parse_cpu_limit("max 100000"), CpuLimit::Unlimited);
        assert_eq!(parse_cpu_limit("150000 100000"), CpuLimit::Limited(1.5));
        assert_eq!(parse_cpu_limit("1 0"), CpuLimit::Unknown);
        assert_eq!(parse_cpu_limit("max"), CpuLimit::Unknown);
        assert_eq!(parse_cpu_limit("0 100000"), CpuLimit::Unknown);
        assert_eq!(parse_cpu_limit("max 100000 extra"), CpuLimit::Unknown);
    }

    #[test]
    fn diagnostic_serialization_is_truthful_about_gpu_telemetry() {
        let report = SystemReport::collect();
        let json = serde_json::to_value(&report).expect("report JSON");
        assert!(json["gpu_vram_bytes"].is_null());
        assert!(report.notice.contains("No provider connections"));
    }
}
