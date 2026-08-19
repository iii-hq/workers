use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::ssrf::{check_target, parse_target, SsrfPolicy};

const MAX_HEADER_BYTES: usize = 64 * 1024;

/// Local HTTP/CONNECT proxy used by safe-mode Chromium. Every connection is
/// resolved, checked, and then pinned to the checked address before dialing.
pub struct EgressGate {
    address: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl EgressGate {
    pub async fn start(policy: SsrfPolicy) -> Result<Self, String> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|e| format!("starting browser egress gate: {e}"))?;
        let address = listener
            .local_addr()
            .map_err(|e| format!("reading browser egress gate address: {e}"))?;
        let (stop, mut stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stopped => break,
                    accepted = listener.accept() => match accepted {
                        Ok((socket, _)) => {
                            tokio::spawn(handle(socket, policy));
                        }
                        Err(_) => break,
                    }
                }
            }
        });
        Ok(Self {
            address,
            stop: Some(stop),
            task: Some(task),
        })
    }

    pub fn proxy_url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub async fn close(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for EgressGate {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn handle(mut client: TcpStream, policy: SsrfPolicy) {
    if let Err((status, detail)) = proxy(&mut client, policy).await {
        let body = format!("browser egress denied: {detail}\n");
        let _ = client
            .write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
    }
}

async fn proxy(client: &mut TcpStream, policy: SsrfPolicy) -> Result<(), (&'static str, String)> {
    let request = read_head(client).await?;
    let split = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
        .ok_or_else(|| ("400 Bad Request", "incomplete request headers".to_string()))?;
    let head = std::str::from_utf8(&request[..split]).map_err(|_| {
        (
            "400 Bad Request",
            "request headers are not UTF-8".to_string(),
        )
    })?;
    let first_end = head
        .find("\r\n")
        .ok_or_else(|| ("400 Bad Request", "missing request line".to_string()))?;
    let mut request_line = head[..first_end].split_whitespace();
    let method = request_line.next().unwrap_or_default();
    let target = request_line.next().unwrap_or_default();
    let version = request_line.next().unwrap_or_default();
    if request_line.next().is_some() || !version.starts_with("HTTP/") {
        return Err(("400 Bad Request", "invalid request line".to_string()));
    }

    let (parsed, connect) = if method.eq_ignore_ascii_case("CONNECT") {
        (parse_target(&format!("https://{target}/")), true)
    } else {
        (parse_target(target), false)
    };
    let parsed = parsed.map_err(|e| ("400 Bad Request", e))?;
    let resolved = check_target(&parsed, &policy)
        .await
        .map_err(|e| ("403 Forbidden", e.message))?;
    let mut upstream = TcpStream::connect(SocketAddr::new(resolved.address, resolved.port))
        .await
        .map_err(|e| {
            (
                "502 Bad Gateway",
                format!("connecting to {}: {e}", parsed.hostname),
            )
        })?;

    if connect {
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .map_err(|e| ("502 Bad Gateway", e.to_string()))?;
    } else {
        let path = match parsed.url.query() {
            Some(query) => format!("{}?{query}", parsed.url.path()),
            None => parsed.url.path().to_string(),
        };
        // The upstream socket is pinned to the FIRST validated host; only one
        // request may ever travel on it. Strip the client's connection
        // management and force `Connection: close` so a keep-alive client
        // cannot send a second (differently-addressed) request down this
        // pinned socket.
        let mut forwarded = format!("{method} {path} {version}\r\n");
        for line in head[first_end + 2..].split("\r\n") {
            if line.is_empty() {
                continue;
            }
            let name = line.split(':').next().unwrap_or_default().trim();
            if name.eq_ignore_ascii_case("connection")
                || name.eq_ignore_ascii_case("proxy-connection")
                || name.eq_ignore_ascii_case("keep-alive")
            {
                continue;
            }
            forwarded.push_str(line);
            forwarded.push_str("\r\n");
        }
        forwarded.push_str("Connection: close\r\n\r\n");
        upstream
            .write_all(forwarded.as_bytes())
            .await
            .map_err(|e| ("502 Bad Gateway", e.to_string()))?;
    }
    if request.len() > split {
        upstream
            .write_all(&request[split..])
            .await
            .map_err(|e| ("502 Bad Gateway", e.to_string()))?;
    }
    tokio::io::copy_bidirectional(client, &mut upstream)
        .await
        .map_err(|e| ("502 Bad Gateway", e.to_string()))?;
    Ok(())
}

async fn read_head(client: &mut TcpStream) -> Result<Vec<u8>, (&'static str, String)> {
    let mut request = Vec::with_capacity(1024);
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        if request.len() == MAX_HEADER_BYTES {
            return Err((
                "431 Request Header Fields Too Large",
                "request headers exceed 64 KiB".to_string(),
            ));
        }
        let start = request.len();
        let end = (start + 4096).min(MAX_HEADER_BYTES);
        request.resize(end, 0);
        let read = client
            .read(&mut request[start..end])
            .await
            .map_err(|e| ("400 Bad Request", e.to_string()))?;
        request.truncate(start + read);
        if read == 0 {
            return Err((
                "400 Bad Request",
                "connection closed before headers".to_string(),
            ));
        }
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn proxies_http_and_blocks_metadata_connects() {
        let origin = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let origin_address = origin.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = origin.accept().await.unwrap();
            let request = read_head(&mut socket).await.unwrap();
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("GET /path?q=1 HTTP/1.1\r\n"));
            // Exactly one Connection header, forced to close.
            assert_eq!(request.matches("Connection:").count(), 1);
            assert!(request.contains("\r\nConnection: close\r\n"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        let gate = EgressGate::start(SsrfPolicy {
            allow_loopback: true,
        })
        .await
        .unwrap();
        let gate_address = gate.address;
        let mut client = TcpStream::connect(gate_address).await.unwrap();
        client
            .write_all(
                format!(
                    "GET http://{origin_address}/path?q=1 HTTP/1.1\r\nHost: {origin_address}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        assert!(response.ends_with("\r\n\r\nok"));
        server.await.unwrap();

        let mut client = TcpStream::connect(gate_address).await.unwrap();
        client
            .write_all(b"CONNECT 169.254.169.254:80 HTTP/1.1\r\nHost: 169.254.169.254\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 403 Forbidden\r\n"));
        gate.close().await;
    }
}
