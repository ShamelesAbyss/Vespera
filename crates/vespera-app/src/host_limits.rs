use serde::Serialize;
use sysinfo::{MemoryRefreshKind, System};
use vespera_core::ResourceBudget;

/// Startup host observations and the resulting scheduler ceilings.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct LocalHostLimits {
    pub available_ram_bytes: u64,
    pub process_cgroup_ram_headroom_bytes: Option<u64>,
    pub available_parallelism: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nvidia_gpu: Option<NvidiaGpuMemory>,
    pub effective_budget: ResourceBudget,
}

/// Observed memory on the NVIDIA device selected by the operator.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct NvidiaGpuMemory {
    pub total_vram_bytes: u64,
    pub free_vram_bytes: u64,
}

impl LocalHostLimits {
    pub(crate) fn collect(configured: ResourceBudget) -> Option<Self> {
        let mut host = System::new();
        host.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        if host.total_memory() == 0 {
            return None;
        }
        let cpu = std::thread::available_parallelism().ok()?;
        let parallelism = u32::try_from(cpu.get()).unwrap_or(u32::MAX);
        #[cfg(target_os = "linux")]
        let cgroup_headroom = process_cgroup_headroom(&mut host);
        #[cfg(not(target_os = "linux"))]
        let cgroup_headroom = None;
        Self::from_observations(
            configured,
            host.available_memory(),
            cgroup_headroom,
            parallelism,
        )
    }

    fn from_observations(
        configured: ResourceBudget,
        available_ram_bytes: u64,
        process_cgroup_ram_headroom_bytes: Option<u64>,
        available_parallelism: u32,
    ) -> Option<Self> {
        let ram_bytes = configured.ram_bytes.min(available_ram_bytes);
        let effective_budget = ResourceBudget {
            ram_bytes: ram_bytes.min(process_cgroup_ram_headroom_bytes.unwrap_or(u64::MAX)),
            max_concurrent_inferences: configured
                .max_concurrent_inferences
                .min(available_parallelism),
            ..configured
        };
        effective_budget.validate().ok()?;
        Some(Self {
            available_ram_bytes,
            process_cgroup_ram_headroom_bytes,
            available_parallelism,
            nvidia_gpu: None,
            effective_budget,
        })
    }

    pub(crate) fn with_nvidia_gpu(mut self, memory: NvidiaGpuMemory) -> Option<Self> {
        if memory.total_vram_bytes == 0 || memory.free_vram_bytes > memory.total_vram_bytes {
            return None;
        }
        self.effective_budget.vram_bytes =
            self.effective_budget.vram_bytes.min(memory.free_vram_bytes);
        self.nvidia_gpu = Some(memory);
        Some(self)
    }
}

impl NvidiaGpuMemory {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub(crate) fn collect(uuid: &str) -> Option<Self> {
        #[cfg(target_os = "windows")]
        let library = {
            let root = std::path::PathBuf::from(std::env::var_os("SystemRoot")?);
            if !root.is_absolute() {
                return None;
            }
            root.join("System32").join("nvml.dll")
        };
        #[cfg(target_os = "linux")]
        let library = std::path::Path::new("libnvidia-ml.so.1");
        let nvml = nvml_wrapper::Nvml::builder()
            .lib_path(library.as_os_str())
            .init()
            .ok()?;
        let device = nvml.device_by_uuid(uuid).ok()?;
        let memory = device.memory_info().ok()?;
        Some(Self {
            total_vram_bytes: memory.total,
            free_vram_bytes: memory.free,
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    pub(crate) fn collect(_uuid: &str) -> Option<Self> {
        None
    }
}
#[cfg(target_os = "linux")]
fn process_cgroup_headroom(host: &mut System) -> Option<u64> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};

    let pid = sysinfo::get_current_pid().ok()?;
    host.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing(),
    );
    Some(host.process(pid)?.cgroup_limits()?.free_memory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_limits_only_reduce_ram_and_slots() {
        let declared = ResourceBudget {
            ram_bytes: 4096,
            vram_bytes: 8192,
            max_concurrent_inferences: 8,
            max_context_tokens: 512,
        };
        let limits = LocalHostLimits::from_observations(declared, 2048, Some(1024), 4).unwrap();
        assert_eq!(
            limits.effective_budget,
            ResourceBudget {
                ram_bytes: 1024,
                max_concurrent_inferences: 4,
                ..declared
            }
        );
        assert_eq!(
            LocalHostLimits::from_observations(declared, 8192, None, 16)
                .unwrap()
                .effective_budget,
            declared
        );
        assert!(LocalHostLimits::from_observations(declared, 0, None, 4).is_none());
        assert!(LocalHostLimits::from_observations(declared, 2048, Some(0), 4).is_none());
        assert!(LocalHostLimits::from_observations(declared, 2048, None, 0).is_none());
    }

    #[test]
    fn nvidia_memory_tightens_vram_and_preserves_cpu_only_admission() {
        let declared = ResourceBudget {
            vram_bytes: 8192,
            ..ResourceBudget::default()
        };
        let host =
            LocalHostLimits::from_observations(declared, declared.ram_bytes, None, 4).unwrap();
        for (free, expected) in [(0, 0), (4096, 4096), (16384, 8192)] {
            let constrained = host
                .with_nvidia_gpu(NvidiaGpuMemory {
                    total_vram_bytes: 16384,
                    free_vram_bytes: free,
                })
                .unwrap();
            assert_eq!(
                constrained.effective_budget,
                ResourceBudget {
                    vram_bytes: expected,
                    ..declared
                }
            );
        }
        assert!(
            host.with_nvidia_gpu(NvidiaGpuMemory {
                total_vram_bytes: 1024,
                free_vram_bytes: 2048,
            })
            .is_none()
        );
    }
}
