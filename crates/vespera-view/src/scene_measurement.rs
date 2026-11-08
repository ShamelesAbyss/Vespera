use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::time::Real;
use bevy::window::PrimaryWindow;
use serde_json::json;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use crate::atlas::{Layer, Layers};

const WARMUP: usize = 120;

#[derive(Resource)]
struct Measurement {
    count: usize,
    context: Option<(Layer, UVec2)>,
    warmup_left: usize,
    intervals_ms: Vec<f64>,
}

pub(super) fn install(app: &mut App, count: usize) {
    app.insert_resource(Measurement {
        count,
        context: None,
        warmup_left: WARMUP,
        intervals_ms: Vec::with_capacity(count),
    })
    .add_systems(Last, measure);
}

fn measure(
    mut sample: ResMut<Measurement>,
    time: Res<Time<Real>>,
    window: Single<&Window, With<PrimaryWindow>>,
    layers: Res<Layers>,
    adapter: Res<RenderAdapterInfo>,
) {
    let context = window
        .focused
        .then_some((layers.active, window.physical_size()));
    if sample.context != context {
        sample.context = context;
        sample.warmup_left = WARMUP;
        sample.intervals_ms.clear();
    }
    if context.is_none() || sample.intervals_ms.len() == sample.count {
        return;
    }
    if sample.warmup_left > 0 {
        sample.warmup_left -= 1;
        return;
    }
    sample.intervals_ms.push(time.delta_secs_f64() * 1000.0);
    if sample.intervals_ms.len() != sample.count {
        return;
    }
    sample.intervals_ms.sort_by(f64::total_cmp);
    let intervals = &sample.intervals_ms;
    println!(
        "{}",
        json!({
            "kind": "atlas_update_sample",
            "pid": std::process::id(),
            "layer": layers.active.name(),
            "adapter": adapter.name,
            "backend": format!("{:?}", adapter.backend),
            "width_px": window.physical_width(),
            "height_px": window.physical_height(),
            "present_mode": format!("{:?}", window.present_mode),
            "warmup_updates": WARMUP,
            "updates": sample.count,
            "update_interval_ms": {
                "p50": percentile(intervals, 50),
                "p95": percentile(intervals, 95),
                "p99": percentile(intervals, 99),
                "max": intervals[intervals.len() - 1]
            },
            "process_resident_bytes": process_resident_bytes(),
        })
    );
}

fn percentile(sorted: &[f64], percent: usize) -> f64 {
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}

fn process_resident_bytes() -> Option<u64> {
    let pid = sysinfo::get_current_pid().ok()?;
    let mut host = System::new();
    host.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing().with_memory(),
    );
    host.process(pid)
        .map(|process| process.memory())
        .filter(|&bytes| bytes > 0)
}

#[cfg(test)]
mod tests {
    use super::percentile;

    #[test]
    fn reports_observed_intervals_by_nearest_rank() {
        let samples = (1..=60).map(f64::from).collect::<Vec<_>>();
        assert_eq!(percentile(&samples, 50), 30.0);
        assert_eq!(percentile(&samples, 95), 57.0);
        assert_eq!(percentile(&samples, 99), 60.0);
    }
}
