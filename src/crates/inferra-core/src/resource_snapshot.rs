//! Host resource snapshots and short timed runtime monitoring for AI bundles.

use serde_json::{json, Value};
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, RefreshKind, System};
use time::OffsetDateTime;

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

/// One-shot CPU/RAM/swap/disk/process summary for investigation bundles.
pub fn collect_host_resources_snapshot() -> Value {
    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_memory(MemoryRefreshKind::everything())
            .with_cpu(CpuRefreshKind::everything()),
    );
    sys.refresh_memory();
    sys.refresh_cpu_usage();

    let load = System::load_average();
    let disks = Disks::new_with_refreshed_list();

    let disk_summaries: Vec<Value> = disks
        .list()
        .iter()
        .take(12)
        .map(|d| {
            json!({
                "mount": d.mount_point().to_string_lossy(),
                "total_bytes": d.total_space(),
                "available_bytes": d.available_space(),
            })
        })
        .collect();

    let total_mem = sys.total_memory();
    let used_mem = sys.used_memory();
    let used_ratio = if total_mem > 0 {
        used_mem as f64 / total_mem as f64
    } else {
        0.0
    };

    json!({
        "captured_at": now_rfc3339(),
        "hostname": System::host_name(),
        "cpus": sys.cpus().len(),
        "global_cpu_percent": sys.global_cpu_usage(),
        "total_memory_bytes": total_mem,
        "used_memory_bytes": used_mem,
        "used_memory_ratio": used_ratio,
        "total_swap_bytes": sys.total_swap(),
        "used_swap_bytes": sys.used_swap(),
        "load_average": {
            "one": load.one,
            "five": load.five,
            "fifteen": load.fifteen,
        },
        "process_count": sys.processes().len(),
        "disks": disk_summaries,
    })
}

/// Sample global CPU and memory at `interval_ms` for `window_secs` (minimum 1s of wall time).
pub async fn collect_runtime_monitor_window(window_secs: u64, interval_ms: u64) -> Value {
    let window_secs = window_secs.max(1);
    let interval_ms = interval_ms.clamp(200, 10_000);
    let interval = std::time::Duration::from_millis(interval_ms);

    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_memory(MemoryRefreshKind::everything())
            .with_cpu(CpuRefreshKind::everything()),
    );
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    tokio::time::sleep(interval).await;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(window_secs);
    let mut samples: Vec<Value> = Vec::new();

    while tokio::time::Instant::now() < deadline {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        let total_mem = sys.total_memory();
        let used_mem = sys.used_memory();
        samples.push(json!({
            "t": now_rfc3339(),
            "global_cpu_percent": sys.global_cpu_usage(),
            "used_memory_bytes": used_mem,
            "total_memory_bytes": total_mem,
            "used_memory_ratio": if total_mem > 0 { used_mem as f64 / total_mem as f64 } else { 0.0 },
            "used_swap_bytes": sys.used_swap(),
            "total_swap_bytes": sys.total_swap(),
        }));
        tokio::time::sleep(interval).await;
    }

    let n = samples.len().max(1) as f64;
    let avg_cpu: f64 = samples
        .iter()
        .filter_map(|s| s.get("global_cpu_percent").and_then(|v| v.as_f64()))
        .sum::<f64>()
        / n;
    let max_cpu = samples
        .iter()
        .filter_map(|s| s.get("global_cpu_percent").and_then(|v| v.as_f64()))
        .fold(0.0_f64, f64::max);
    let avg_mem_ratio: f64 = samples
        .iter()
        .filter_map(|s| s.get("used_memory_ratio").and_then(|v| v.as_f64()))
        .sum::<f64>()
        / n;

    json!({
        "window_seconds": window_secs,
        "sample_interval_ms": interval_ms,
        "sample_count": samples.len(),
        "samples": samples,
        "summary": {
            "avg_global_cpu_percent": avg_cpu,
            "max_global_cpu_percent": max_cpu,
            "avg_used_memory_ratio": avg_mem_ratio,
        },
    })
}

/// Optional NVIDIA GPU stats when `nvidia-smi` is on PATH (read-only, 2s cap).
pub async fn try_collect_gpu_summary() -> Value {
    let out = match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::process::Command::new("nvidia-smi")
            .args([
                "--query-gpu=name,memory.used,memory.total,utilization.gpu",
                "--format=csv,noheader,nounits",
            ])
            .output(),
    )
    .await
    {
        Ok(Ok(o)) if o.status.success() => o.stdout,
        _ => return Value::Null,
    };
    let line = String::from_utf8_lossy(&out);
    let first = line.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        return Value::Null;
    }
    let parts: Vec<&str> = first.split(',').map(|s| s.trim()).collect();
    if parts.len() < 4 {
        return Value::Null;
    }
    json!({
        "source": "nvidia-smi",
        "name": parts[0],
        "memory_used_mib": parts.get(1).and_then(|s| s.parse::<f64>().ok()),
        "memory_total_mib": parts.get(2).and_then(|s| s.parse::<f64>().ok()),
        "utilization_gpu_percent": parts.get(3).and_then(|s| s.parse::<f64>().ok()),
    })
}
