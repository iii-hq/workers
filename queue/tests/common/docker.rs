#![allow(dead_code)]
//! Docker-backed Redis/RabbitMQ containers for the redis/rabbitmq adapter
//! e2e tests.
//!
//! Skips (returns `None`) whenever `docker info` fails — no Docker daemon
//! reachable — so CI (which runs without Docker) and casual local runs stay
//! green. Each container is started with an ephemeral host port (`-p 0:<port>`,
//! so parallel test runs never collide on a fixed port) and torn down via
//! `Drop` (`docker stop`, which also removes it since it's started with
//! `--rm`).

use std::process::Command;

pub struct RedisContainer {
    container_id: String,
    pub host_port: u16,
}

impl RedisContainer {
    pub fn redis_url(&self) -> String {
        format!("redis://127.0.0.1:{}", self.host_port)
    }
}

impl Drop for RedisContainer {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["stop", "-t", "1", &self.container_id])
            .output();
    }
}

fn docker_available() -> bool {
    Command::new("docker")
        .arg("info")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Starts a `redis:7-alpine` container on an ephemeral host port. Returns
/// `None` (the caller should skip the test) when Docker isn't reachable or
/// the container fails to start/become ready.
pub fn start_redis() -> Option<RedisContainer> {
    if !docker_available() {
        eprintln!("[skip] docker not reachable — skipping redis adapter e2e test");
        return None;
    }

    let run_output = Command::new("docker")
        .args(["run", "-d", "--rm", "-p", "0:6379", "redis:7-alpine"])
        .output()
        .ok()?;

    if !run_output.status.success() {
        eprintln!(
            "[skip] `docker run` failed: {}",
            String::from_utf8_lossy(&run_output.stderr)
        );
        return None;
    }

    let container_id = String::from_utf8_lossy(&run_output.stdout)
        .trim()
        .to_string();
    if container_id.is_empty() {
        return None;
    }

    let host_port = resolve_mapped_port(&container_id, "6379/tcp")?;

    let container = RedisContainer {
        container_id,
        host_port,
    };

    if !wait_until_tcp_ready(container.host_port) {
        eprintln!("[skip] redis container did not become ready in time");
        return None;
    }

    Some(container)
}

/// `docker port <id> <port_spec>` prints e.g. `0.0.0.0:54321` (and often a
/// second `[::]:54321` line for the IPv6 binding) — take the first line's
/// port.
fn resolve_mapped_port(container_id: &str, port_spec: &str) -> Option<u16> {
    let port_output = Command::new("docker")
        .args(["port", container_id, port_spec])
        .output()
        .ok()?;
    if !port_output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&port_output.stdout);
    let first_line = stdout.lines().next()?;
    let port_str = first_line.rsplit(':').next()?;
    port_str.trim().parse::<u16>().ok()
}

fn wait_until_tcp_ready(host_port: u16) -> bool {
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    let addr = format!("127.0.0.1:{}", host_port);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect(&addr).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Docker-backed RabbitMQ container for the rabbitmq adapter e2e tests.
///
/// RabbitMQ boots noticeably slower than Redis (~10-20s): the TCP port opens
/// well before the AMQP protocol handshake is actually served, so readiness
/// is checked with a real `lapin::Connection::connect` retry loop instead of
/// a bare TCP connect (see `wait_until_tcp_ready`, used for Redis).
#[cfg(feature = "rabbitmq")]
pub struct RabbitMqContainer {
    container_id: String,
    pub host_port: u16,
}

#[cfg(feature = "rabbitmq")]
impl RabbitMqContainer {
    pub fn amqp_url(&self) -> String {
        format!("amqp://127.0.0.1:{}/%2f", self.host_port)
    }
}

#[cfg(feature = "rabbitmq")]
impl Drop for RabbitMqContainer {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["stop", "-t", "1", &self.container_id])
            .output();
    }
}

/// Starts a `rabbitmq:3-alpine` container on an ephemeral host port. Returns
/// `None` (the caller should skip the test) when Docker isn't reachable or
/// the container fails to start/become ready within the deadline.
#[cfg(feature = "rabbitmq")]
pub async fn start_rabbitmq() -> Option<RabbitMqContainer> {
    if !docker_available() {
        eprintln!("[skip] docker not reachable — skipping rabbitmq adapter e2e test");
        return None;
    }

    let run_output = Command::new("docker")
        .args(["run", "-d", "--rm", "-p", "0:5672", "rabbitmq:3-alpine"])
        .output()
        .ok()?;

    if !run_output.status.success() {
        eprintln!(
            "[skip] `docker run` failed: {}",
            String::from_utf8_lossy(&run_output.stderr)
        );
        return None;
    }

    let container_id = String::from_utf8_lossy(&run_output.stdout)
        .trim()
        .to_string();
    if container_id.is_empty() {
        return None;
    }

    let host_port = resolve_mapped_port(&container_id, "5672/tcp")?;
    let container = RabbitMqContainer {
        container_id,
        host_port,
    };

    if !wait_until_amqp_ready(&container).await {
        eprintln!("[skip] rabbitmq container did not become ready in time");
        return None;
    }

    Some(container)
}

#[cfg(feature = "rabbitmq")]
async fn wait_until_amqp_ready(container: &RabbitMqContainer) -> bool {
    use tokio::time::{sleep, Duration, Instant};

    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match lapin::Connection::connect(
            &container.amqp_url(),
            lapin::ConnectionProperties::default(),
        )
        .await
        {
            Ok(connection) => {
                let _ = connection.close(200, "readiness check").await;
                return true;
            }
            Err(_) => sleep(Duration::from_millis(300)).await,
        }
    }
    false
}
