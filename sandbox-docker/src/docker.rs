use anyhow::{anyhow, Result};
use bollard::container::{Config, CreateContainerOptions, LogOutput, RemoveContainerOptions};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::image::CreateImageOptions;
use bollard::Docker;
use bytes::Bytes;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::time::Instant;

use crate::types::{ExecResult, FileInfo};

const OUTPUT_CAP: usize = 10 * 1024 * 1024;
const CONTAINER_PREFIX: &str = "iii-sbx";

pub fn connect_docker() -> Result<Docker> {
    Docker::connect_with_local_defaults().map_err(|e| anyhow!("failed to connect to Docker: {e}"))
}

pub async fn ensure_image(docker: &Docker, image: &str) -> Result<()> {
    let mut stream = docker.create_image(
        Some(CreateImageOptions {
            from_image: image,
            ..Default::default()
        }),
        None,
        None,
    );

    while let Some(result) = stream.next().await {
        match result {
            Ok(info) => {
                if let Some(status) = &info.status {
                    tracing::debug!(status = %status, "pulling image");
                }
            }
            Err(e) => return Err(anyhow!("failed to pull image {image}: {e}")),
        }
    }

    Ok(())
}

pub struct ContainerOpts<'a> {
    pub id: &'a str,
    pub image: &'a str,
    pub memory_mb: u64,
    pub cpu: f64,
    pub network: bool,
    pub env: &'a HashMap<String, String>,
    pub workdir: &'a str,
}

pub async fn create_container(docker: &Docker, opts: &ContainerOpts<'_>) -> Result<String> {
    let id = opts.id;
    let image = opts.image;
    let memory_mb = opts.memory_mb;
    let cpu = opts.cpu;
    let network = opts.network;
    let env = opts.env;
    let workdir = opts.workdir;
    let container_name = format!("{CONTAINER_PREFIX}-{id}");

    let env_vec: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();

    let nano_cpus = (cpu * 1_000_000_000.0) as i64;
    let memory_bytes = (memory_mb as i64) * 1024 * 1024;

    let network_mode = if network {
        None
    } else {
        Some("none".to_string())
    };

    let host_config = bollard::models::HostConfig {
        memory: Some(memory_bytes),
        nano_cpus: Some(nano_cpus),
        pids_limit: Some(256),
        cap_drop: Some(vec![
            "NET_RAW".to_string(),
            "SYS_ADMIN".to_string(),
            "MKNOD".to_string(),
        ]),
        security_opt: Some(vec!["no-new-privileges:true".to_string()]),
        network_mode,
        ..Default::default()
    };

    let config = Config {
        image: Some(image.to_string()),
        cmd: Some(vec![
            "tail".to_string(),
            "-f".to_string(),
            "/dev/null".to_string(),
        ]),
        working_dir: Some(workdir.to_string()),
        env: Some(env_vec),
        host_config: Some(host_config),
        ..Default::default()
    };

    docker
        .create_container(
            Some(CreateContainerOptions {
                name: container_name.clone(),
                ..Default::default()
            }),
            config,
        )
        .await
        .map_err(|e| anyhow!("failed to create container: {e}"))?;

    docker
        .start_container::<String>(&container_name, None)
        .await
        .map_err(|e| anyhow!("failed to start container: {e}"))?;

    Ok(container_name)
}

pub async fn exec_in_container(
    docker: &Docker,
    container_name: &str,
    command: Vec<String>,
    timeout_ms: u64,
) -> Result<ExecResult> {
    let exec = docker
        .create_exec(
            container_name,
            CreateExecOptions {
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                cmd: Some(command),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| anyhow!("failed to create exec: {e}"))?;

    let start = Instant::now();
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut total_bytes = 0usize;

    let output = docker
        .start_exec(&exec.id, None)
        .await
        .map_err(|e| anyhow!("failed to start exec: {e}"))?;

    if let StartExecResults::Attached { mut output, .. } = output {
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);

        loop {
            let next = tokio::time::timeout_at(deadline, output.next()).await;
            match next {
                Ok(Some(Ok(msg))) => {
                    let chunk = match &msg {
                        LogOutput::StdOut { message } => {
                            let s = String::from_utf8_lossy(message);
                            stdout.push_str(&s);
                            message.len()
                        }
                        LogOutput::StdErr { message } => {
                            let s = String::from_utf8_lossy(message);
                            stderr.push_str(&s);
                            message.len()
                        }
                        _ => 0,
                    };
                    total_bytes += chunk;
                    if total_bytes > OUTPUT_CAP {
                        stderr.push_str("\n[output truncated: 10MB limit exceeded]");
                        break;
                    }
                }
                Ok(Some(Err(e))) => {
                    return Err(anyhow!("exec stream error: {e}"));
                }
                Ok(None) => break,
                Err(_) => {
                    stderr.push_str("\n[timeout exceeded]");
                    break;
                }
            }
        }
    }

    let inspect = docker
        .inspect_exec(&exec.id)
        .await
        .map_err(|e| anyhow!("failed to inspect exec: {e}"))?;

    let exit_code = inspect.exit_code.unwrap_or(-1);
    let duration_ms = start.elapsed().as_millis() as u64;

    Ok(ExecResult {
        exit_code,
        stdout,
        stderr,
        duration_ms,
    })
}

pub async fn copy_to_container(
    docker: &Docker,
    container_name: &str,
    path: &str,
    content: &[u8],
) -> Result<()> {
    let parent = std::path::Path::new(path)
        .parent()
        .unwrap_or(std::path::Path::new("/"));
    let filename = std::path::Path::new(path)
        .file_name()
        .ok_or_else(|| anyhow!("invalid file path: {path}"))?
        .to_string_lossy()
        .to_string();

    let mut archive = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_path(&filename)?;
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive.append(&header, content)?;
    let tar_bytes = archive.into_inner()?;

    docker
        .upload_to_container(
            container_name,
            Some(bollard::container::UploadToContainerOptions {
                path: parent.to_string_lossy().to_string(),
                ..Default::default()
            }),
            Bytes::from(tar_bytes),
        )
        .await
        .map_err(|e| anyhow!("failed to copy file to container: {e}"))?;

    Ok(())
}

pub async fn read_file(
    docker: &Docker,
    container_name: &str,
    path: &str,
    timeout_ms: u64,
) -> Result<String> {
    let result = exec_in_container(
        docker,
        container_name,
        vec!["cat".to_string(), path.to_string()],
        timeout_ms,
    )
    .await?;

    if result.exit_code != 0 {
        return Err(anyhow!("failed to read file {path}: {}", result.stderr));
    }

    Ok(result.stdout)
}

pub async fn list_dir(
    docker: &Docker,
    container_name: &str,
    path: &str,
    timeout_ms: u64,
) -> Result<Vec<FileInfo>> {
    let result = exec_in_container(
        docker,
        container_name,
        vec![
            "find".to_string(),
            path.to_string(),
            "-maxdepth".to_string(),
            "1".to_string(),
            "-printf".to_string(),
            "%y|%s|%f|%p\\n".to_string(),
        ],
        timeout_ms,
    )
    .await?;

    if result.exit_code != 0 {
        return Err(anyhow!(
            "failed to list directory {path}: {}",
            result.stderr
        ));
    }

    let mut entries = Vec::new();
    for line in result.stdout.lines() {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() < 4 {
            continue;
        }
        let is_directory = parts[0] == "d";
        let size = parts[1].parse::<u64>().unwrap_or(0);
        let name = parts[2].to_string();
        let entry_path = parts[3].to_string();

        if name == "." || name == ".." {
            continue;
        }

        entries.push(FileInfo {
            name,
            path: entry_path,
            size,
            is_directory,
        });
    }

    Ok(entries)
}

pub async fn stop_and_remove(docker: &Docker, id: &str) -> Result<()> {
    let container_name = format!("{CONTAINER_PREFIX}-{id}");

    let _ = docker.stop_container(&container_name, None).await;

    docker
        .remove_container(
            &container_name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .map_err(|e| anyhow!("failed to remove container {container_name}: {e}"))?;

    Ok(())
}
