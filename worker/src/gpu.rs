//! Boot-time GPU diagnostic. `agents/gpu-compute.json`'s system prompt tells
//! the agent to "check nvidia-smi before assuming the GPU is free" — this just
//! logs that same reading once at worker startup so a wedged GPU shows up in
//! the worker's own logs, not only inside a session transcript. Never fatal:
//! the worker still claims and runs non-GPU tool calls without a GPU present.

pub async fn log_status() {
    match tokio::process::Command::new("nvidia-smi")
        .arg("--query-gpu=name,memory.used,memory.total,utilization.gpu")
        .arg("--format=csv,noheader")
        .output()
        .await
    {
        Ok(out) if out.status.success() => {
            let reading = String::from_utf8_lossy(&out.stdout);
            for line in reading.lines() {
                tracing::info!(gpu = %line.trim(), "nvidia-smi");
            }
        }
        Ok(out) => {
            tracing::warn!(
                stderr = %String::from_utf8_lossy(&out.stderr).trim(),
                "nvidia-smi exited non-zero at startup"
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "nvidia-smi not available at startup");
        }
    }
}
