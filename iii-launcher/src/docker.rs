use anyhow::{anyhow, Context, Result};
use std::io::Read;
use tokio::process::Command;

use crate::adapter::{ContainerSpec, ContainerStatus, ImageInfo, RuntimeAdapter};

pub struct DockerAdapter;

impl DockerAdapter {
    pub fn new() -> Self {
        Self
    }

    async fn run_cmd(args: &[&str]) -> Result<String> {
        let output = Command::new("docker")
            .args(args)
            .output()
            .await
            .with_context(|| format!("failed to execute: docker {}", args.join(" ")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!(
                "docker {} failed (exit {}): {}",
                args.join(" "),
                output.status.code().unwrap_or(-1),
                stderr.trim()
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

#[async_trait::async_trait]
impl RuntimeAdapter for DockerAdapter {
    async fn pull(&self, image: &str) -> Result<ImageInfo> {
        tracing::info!(image = %image, "pulling image");
        Self::run_cmd(&["pull", image]).await?;

        // Get image size via docker inspect
        let size_str =
            Self::run_cmd(&["inspect", "--format", "{{.Size}}", image]).await;
        let size_bytes = size_str.ok().and_then(|s| s.parse::<u64>().ok());

        Ok(ImageInfo {
            image: image.to_string(),
            size_bytes,
        })
    }

    async fn extract_file(&self, image: &str, path: &str) -> Result<Vec<u8>> {
        // Create a temporary container (never started) to copy a file out
        let container_name = format!("iii-extract-{}", uuid::Uuid::new_v4());
        Self::run_cmd(&["create", "--name", &container_name, image, "true"]).await?;

        let src = format!("{}:{}", container_name, path);
        let result = Self::run_cmd(&["cp", &src, "-"]).await;

        // Always clean up the temp container
        let _ = Self::run_cmd(&["rm", "-f", &container_name]).await;

        let tar_bytes = result.map(|s| s.into_bytes())?;

        // docker cp outputs a tar archive; extract the first entry
        let mut archive = tar::Archive::new(tar_bytes.as_slice());
        for entry in archive.entries()? {
            let mut entry = entry?;
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }

        Err(anyhow!("no files found in tar archive from docker cp"))
    }

    async fn start(&self, spec: &ContainerSpec) -> Result<String> {
        let mut args: Vec<String> = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            spec.name.clone(),
        ];

        for (key, val) in &spec.env {
            args.push("-e".to_string());
            args.push(format!("{}={}", key, val));
        }

        if let Some(ref mem) = spec.memory_limit {
            args.push("--memory".to_string());
            args.push(mem.clone());
        }

        if let Some(ref cpu) = spec.cpu_limit {
            args.push("--cpus".to_string());
            args.push(cpu.clone());
        }

        args.push(spec.image.clone());

        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let container_id = Self::run_cmd(&args_ref).await?;

        tracing::info!(
            name = %spec.name,
            container_id = %container_id,
            "started container"
        );

        Ok(container_id)
    }

    async fn stop(&self, container_id: &str) -> Result<()> {
        tracing::info!(container_id = %container_id, "stopping container");
        Self::run_cmd(&["stop", container_id]).await?;
        Ok(())
    }

    async fn status(&self, container_id: &str) -> Result<ContainerStatus> {
        let running_str = Self::run_cmd(&[
            "inspect",
            "--format",
            "{{.State.Running}}",
            container_id,
        ])
        .await?;

        let exit_code_str = Self::run_cmd(&[
            "inspect",
            "--format",
            "{{.State.ExitCode}}",
            container_id,
        ])
        .await?;

        let name_str = Self::run_cmd(&[
            "inspect",
            "--format",
            "{{.Name}}",
            container_id,
        ])
        .await?;

        let running = running_str == "true";
        let exit_code = exit_code_str.parse::<i32>().ok();
        // Docker prefixes container names with '/'
        let name = name_str.trim_start_matches('/').to_string();

        Ok(ContainerStatus {
            name,
            container_id: container_id.to_string(),
            running,
            exit_code,
        })
    }

    async fn logs(&self, container_id: &str, _follow: bool) -> Result<String> {
        // We don't support follow in a request/response model; always return tail
        let output = Self::run_cmd(&["logs", "--tail", "100", container_id]).await?;
        Ok(output)
    }

    async fn remove(&self, container_id: &str) -> Result<()> {
        tracing::info!(container_id = %container_id, "removing container");
        Self::run_cmd(&["rm", "-f", container_id]).await?;
        Ok(())
    }
}
