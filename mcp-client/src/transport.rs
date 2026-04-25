use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};

pub enum Transport {
    Stdio(StdioTransport),
    Http(HttpTransport),
    Duplex(DuplexTransport),
}

impl Transport {
    pub fn kind(&self) -> &'static str {
        match self {
            Transport::Stdio(_) => "stdio",
            Transport::Http(_) => "http",
            Transport::Duplex(_) => "duplex",
        }
    }

    pub async fn send_raw(&self, line: &str) -> Result<()> {
        match self {
            Transport::Stdio(t) => t.send_raw(line).await,
            Transport::Http(t) => t.send_raw(line).await,
            Transport::Duplex(t) => t.send_raw(line).await,
        }
    }

    pub fn take_reader(&self) -> Option<mpsc::Receiver<String>> {
        match self {
            Transport::Stdio(t) => t.take_reader(),
            Transport::Http(t) => t.take_reader(),
            Transport::Duplex(t) => t.take_reader(),
        }
    }

    pub async fn shutdown(&self) {
        match self {
            Transport::Stdio(t) => t.shutdown().await,
            Transport::Http(_) => {}
            Transport::Duplex(_) => {}
        }
    }

    pub fn from_duplex(read: DuplexStream, write: DuplexStream) -> Transport {
        Transport::Duplex(DuplexTransport::new(read, write))
    }
}

type FramedReadAny<R> = FramedRead<R, LinesCodec>;
type FramedWriteAny<W> = FramedWrite<W, LinesCodec>;

pub struct StdioTransport {
    _child: Arc<Mutex<Option<Child>>>,
    writer: Arc<Mutex<FramedWriteAny<ChildStdin>>>,
    reader_rx: Mutex<Option<mpsc::Receiver<String>>>,
}

impl StdioTransport {
    pub async fn spawn(bin: &str, args: &[String]) -> Result<StdioTransport> {
        let mut cmd = tokio::process::Command::new(bin);
        cmd.args(args);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::inherit());
        cmd.kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn MCP stdio process: {}", bin))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("child stdin missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("child stdout missing"))?;

        let writer: FramedWriteAny<ChildStdin> = FramedWrite::new(stdin, LinesCodec::new());
        let reader: FramedReadAny<ChildStdout> = FramedRead::new(stdout, LinesCodec::new());
        let (tx, rx) = mpsc::channel::<String>(64);

        spawn_lines_pump(reader, tx);

        Ok(StdioTransport {
            _child: Arc::new(Mutex::new(Some(child))),
            writer: Arc::new(Mutex::new(writer)),
            reader_rx: Mutex::new(Some(rx)),
        })
    }

    pub async fn send_raw(&self, line: &str) -> Result<()> {
        use futures_util::SinkExt;
        let mut w = self.writer.lock().await;
        w.send(line.to_string())
            .await
            .map_err(|e| anyhow!("stdio send failed: {e}"))?;
        Ok(())
    }

    pub fn take_reader(&self) -> Option<mpsc::Receiver<String>> {
        self.reader_rx.try_lock().ok().and_then(|mut g| g.take())
    }

    pub async fn shutdown(&self) {
        let mut guard = self._child.lock().await;
        if let Some(mut child) = guard.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }
}

pub struct HttpTransport {
    client: reqwest::Client,
    url: String,
    session_id: RwLock<Option<String>>,
    reader_rx: Mutex<Option<mpsc::Receiver<String>>>,
    reader_tx: mpsc::Sender<String>,
}

impl HttpTransport {
    pub fn new(url: String) -> HttpTransport {
        let (tx, rx) = mpsc::channel::<String>(64);
        HttpTransport {
            client: reqwest::Client::new(),
            url,
            session_id: RwLock::new(None),
            reader_rx: Mutex::new(Some(rx)),
            reader_tx: tx,
        }
    }

    pub async fn send_raw(&self, line: &str) -> Result<()> {
        let mut req = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(line.to_string());

        if let Some(sid) = self.session_id.read().await.clone() {
            req = req.header("Mcp-Session-Id", sid);
        }

        let resp = req
            .send()
            .await
            .with_context(|| format!("HTTP POST failed: {}", self.url))?;

        if let Some(sid) = resp.headers().get("Mcp-Session-Id") {
            if let Ok(s) = sid.to_str() {
                *self.session_id.write().await = Some(s.to_string());
            }
        }

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("HTTP {} from MCP server: {}", status, body));
        }

        let ct = resp
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        // TODO: Streamable HTTP SSE events not consumed in this PR — only the
        // first JSON message is read. SSE multiplexing belongs to a follow-up.
        if ct.starts_with("application/json") {
            let body = resp.text().await.unwrap_or_default();
            if !body.is_empty() {
                let _ = self.reader_tx.send(body).await;
            }
        } else if ct.starts_with("text/event-stream") {
            let mut stream = resp.bytes_stream();
            let mut buf = String::new();
            while let Some(chunk) = stream.next().await {
                let bytes = chunk.with_context(|| "SSE stream chunk error")?;
                buf.push_str(&String::from_utf8_lossy(&bytes));
                if let Some((event, _rest)) = split_first_sse_event(&buf) {
                    if let Some(json) = extract_data_payload(&event) {
                        let _ = self.reader_tx.send(json).await;
                    }
                    break;
                }
            }
        }

        Ok(())
    }

    pub fn take_reader(&self) -> Option<mpsc::Receiver<String>> {
        self.reader_rx.try_lock().ok().and_then(|mut g| g.take())
    }
}

fn split_first_sse_event(s: &str) -> Option<(String, String)> {
    if let Some(idx) = s.find("\n\n") {
        let event = s[..idx].to_string();
        let rest = s[idx + 2..].to_string();
        Some((event, rest))
    } else {
        None
    }
}

fn extract_data_payload(event: &str) -> Option<String> {
    let mut data_lines = Vec::new();
    for line in event.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }
    if data_lines.is_empty() {
        None
    } else {
        Some(data_lines.join("\n"))
    }
}

pub struct DuplexTransport {
    writer: Arc<Mutex<FramedWriteAny<DuplexStream>>>,
    reader_rx: Mutex<Option<mpsc::Receiver<String>>>,
}

impl DuplexTransport {
    pub fn new(read: DuplexStream, write: DuplexStream) -> DuplexTransport {
        let writer: FramedWriteAny<DuplexStream> = FramedWrite::new(write, LinesCodec::new());
        let reader: FramedReadAny<DuplexStream> = FramedRead::new(read, LinesCodec::new());
        let (tx, rx) = mpsc::channel::<String>(64);
        spawn_lines_pump(reader, tx);
        DuplexTransport {
            writer: Arc::new(Mutex::new(writer)),
            reader_rx: Mutex::new(Some(rx)),
        }
    }

    pub async fn send_raw(&self, line: &str) -> Result<()> {
        use futures_util::SinkExt;
        let mut w = self.writer.lock().await;
        w.send(line.to_string())
            .await
            .map_err(|e| anyhow!("duplex send failed: {e}"))?;
        Ok(())
    }

    pub fn take_reader(&self) -> Option<mpsc::Receiver<String>> {
        self.reader_rx.try_lock().ok().and_then(|mut g| g.take())
    }
}

fn spawn_lines_pump<R>(mut reader: FramedReadAny<R>, tx: mpsc::Sender<String>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        while let Some(line) = reader.next().await {
            match line {
                Ok(l) => {
                    if tx.send(l).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "transport reader error; closing pump");
                    break;
                }
            }
        }
    });
}

// silence unused-import warnings on AsyncWrite when only FramedWrite uses it
#[allow(dead_code)]
fn _async_write_marker<W: AsyncWrite + Unpin>(_w: W) {}
