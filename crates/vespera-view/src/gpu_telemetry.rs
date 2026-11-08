use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};

#[derive(Clone, Default)]
pub(super) struct Gpus {
    pub reported: Option<u32>,
    pub devices: BTreeMap<String, Gpu>,
}

impl Gpus {
    pub fn select(&self, bus: &str) -> Option<(&String, &Gpu)> {
        self.devices
            .get_key_value(bus)
            .or_else(|| self.devices.first_key_value())
    }

    pub fn next(&self, bus: &str) -> Option<&String> {
        let (bus, _) = self.select(bus)?;
        self.devices
            .range::<str, _>((Excluded(bus.as_str()), Unbounded))
            .next()
            .or_else(|| self.devices.first_key_value())
            .map(|(bus, _)| bus)
    }
}

#[derive(Clone)]
pub(super) struct Gpu {
    pub name: Option<String>,
    pub memory: Option<Memory>,
}

#[derive(Clone, Copy)]
pub(super) struct Memory {
    pub used: u64,
    pub free: u64,
    pub total: u64,
}

#[derive(Default)]
pub(super) struct Probe {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    nvml: Option<nvml_wrapper::Nvml>,
}

impl Probe {
    pub fn open() -> Self {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        return Self { nvml: open_nvml() };

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        Self::default()
    }

    pub fn sample(&self) -> Gpus {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Some(nvml) = &self.nvml
            && let Ok(count) = nvml.device_count()
        {
            return Gpus {
                reported: Some(count),
                devices: (0..count.min(16))
                    .filter_map(|index| {
                        let device = nvml.device_by_index(index).ok()?;
                        let bus = device.pci_info().ok()?.bus_id;
                        let name = device.name().ok();
                        let memory = device.memory_info().ok().and_then(|memory| {
                            (memory.total > 0
                                && memory.used <= memory.total
                                && memory.free <= memory.total)
                                .then_some(Memory {
                                    used: memory.used,
                                    free: memory.free,
                                    total: memory.total,
                                })
                        });
                        Some((bus, Gpu { name, memory }))
                    })
                    .collect(),
            };
        }
        Gpus::default()
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn open_nvml() -> Option<nvml_wrapper::Nvml> {
    use std::path::PathBuf;

    #[cfg(target_os = "windows")]
    let library = {
        let root = PathBuf::from(std::env::var_os("SystemRoot")?);
        if !root.is_absolute() {
            return None;
        }
        root.join("System32").join("nvml.dll")
    };
    #[cfg(target_os = "linux")]
    let library = PathBuf::from("libnvidia-ml.so.1");
    nvml_wrapper::Nvml::builder()
        .lib_path(library.as_os_str())
        .init()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an accessible local NVIDIA GPU and NVML"]
    fn live_nvml_reports_a_device_and_memory() {
        let gpus = Probe::open().sample();
        assert!(gpus.reported.unwrap_or(0) > 0);
        let (bus, gpu, memory) = gpus
            .devices
            .iter()
            .find_map(|(bus, gpu)| gpu.memory.map(|memory| (bus, gpu, memory)))
            .expect("one accessible GPU with measured memory");
        println!(
            "{bus} {} used={} free={} total={}",
            gpu.name.as_deref().unwrap_or("Unnamed device"),
            memory.used,
            memory.free,
            memory.total
        );
    }

    #[test]
    fn selection_cycles_by_pci_address_and_recovers_after_removal() {
        let unknown = Gpu {
            name: None,
            memory: None,
        };
        let mut gpus = Gpus {
            reported: Some(3),
            devices: [
                ("02:00.0".into(), unknown.clone()),
                ("01:00.0".into(), unknown),
            ]
            .into_iter()
            .collect(),
        };
        assert_eq!(gpus.select("").unwrap().0, "01:00.0");
        assert_eq!(gpus.next("01:00.0").unwrap(), "02:00.0");
        assert_eq!(gpus.next("02:00.0").unwrap(), "01:00.0");
        gpus.devices.remove("02:00.0");
        assert_eq!(gpus.select("02:00.0").unwrap().0, "01:00.0");
        assert!(gpus.select("02:00.0").unwrap().1.memory.is_none());
    }
}
