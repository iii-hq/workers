#[allow(dead_code)]
#[path = "../src/scrapling/cdp.rs"]
mod cdp;

#[cfg(unix)]
mod unix {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::process::Command;
    use std::sync::mpsc;
    use std::thread;

    use serde_json::{json, Value};

    use super::cdp::{CdpClient, CdpError, EventError, REMOTE_DEBUGGING_PIPE_ARG};

    fn read_frame(stream: &mut UnixStream) -> Option<Value> {
        let mut bytes = Vec::new();
        let mut byte = [0];
        loop {
            match stream.read(&mut byte).unwrap() {
                0 if bytes.is_empty() => return None,
                0 => panic!("truncated CDP frame"),
                _ if byte[0] == 0 => return Some(serde_json::from_slice(&bytes).unwrap()),
                _ => bytes.push(byte[0]),
            }
        }
    }

    fn write_frame(stream: &mut UnixStream, value: &Value) {
        serde_json::to_writer(&mut *stream, value).unwrap();
        stream.write_all(&[0]).unwrap();
        stream.flush().unwrap();
    }

    fn write_chunked_frames(stream: &mut UnixStream, values: &[Value]) {
        let mut bytes = Vec::new();
        for value in values {
            serde_json::to_writer(&mut bytes, value).unwrap();
            bytes.push(0);
        }
        let split = 3.min(bytes.len());
        stream.write_all(&bytes[..split]).unwrap();
        stream.write_all(&bytes[split..]).unwrap();
        stream.flush().unwrap();
    }

    #[tokio::test]
    async fn fake_pipe_frames_routes_cancels_and_tears_down() {
        let (client_commands, mut server_commands) = UnixStream::pair().unwrap();
        let (mut server_events, client_events) = UnixStream::pair().unwrap();
        let (closed_tx, closed_rx) = mpsc::channel();

        let server = thread::spawn(move || {
            let root = read_frame(&mut server_commands).unwrap();
            let page = read_frame(&mut server_commands).unwrap();
            assert_eq!(
                root,
                json!({"id": 1, "method": "Browser.getVersion", "params": {}})
            );
            assert_eq!(
                page,
                json!({"id": 2, "method": "Page.getFrameTree", "params": {}, "sessionId": "page-1"})
            );

            write_chunked_frames(
                &mut server_events,
                &[
                    json!({"method": "Browser.downloadWillBegin", "params": {"guid": "g"}}),
                    json!({"method": "Page.loadEventFired", "params": {"timestamp": 1}, "sessionId": "page-1"}),
                    json!({"id": 2, "result": {"wrong": true}, "sessionId": "page-2"}),
                    json!({"id": 2, "result": {"frameTree": "page"}, "sessionId": "page-1"}),
                    json!({"id": 1, "result": {"product": "Chrome"}}),
                ],
            );

            let cancelled = read_frame(&mut server_commands).unwrap();
            assert_eq!(cancelled["id"], 3);
            write_frame(
                &mut server_events,
                &json!({"id": 3, "result": {"late": true}}),
            );

            let after_cancel = read_frame(&mut server_commands).unwrap();
            assert_eq!(after_cancel["id"], 4);
            write_frame(
                &mut server_events,
                &json!({"id": 4, "error": {"code": -32000, "message": "boom"}}),
            );

            assert!(read_frame(&mut server_commands).is_none());
            drop(server_events);
            closed_tx.send(()).unwrap();
        });

        let child = Command::new("sh")
            .args(["-c", "exec sleep 60"])
            .spawn()
            .unwrap();
        let client = CdpClient::from_pipe(client_events, client_commands, Some(child)).unwrap();
        let mut browser_events = client.subscribe();
        let page = client.session("page-1");
        let mut page_events = page.subscribe();

        let root_call = client.send("Browser.getVersion", json!({})).unwrap();
        let page_call = page.send("Page.getFrameTree", json!({})).unwrap();
        let (root_result, page_result) = tokio::join!(root_call, page_call);
        assert_eq!(root_result.unwrap(), json!({"product": "Chrome"}));
        assert_eq!(page_result.unwrap(), json!({"frameTree": "page"}));

        let browser_event = browser_events.recv().await.unwrap();
        assert_eq!(browser_event.method, "Browser.downloadWillBegin");
        assert_eq!(browser_event.session_id, None);
        let page_event = page_events.recv().await.unwrap();
        assert_eq!(page_event.method, "Page.loadEventFired");
        assert_eq!(page_event.session_id.as_deref(), Some("page-1"));

        let cancelled = client
            .send("Runtime.evaluate", json!({"expression": "1"}))
            .unwrap();
        drop(cancelled);
        tokio::task::yield_now().await;
        assert_eq!(client.pending_count(), 0);

        let error = client
            .send("Broken.command", json!({}))
            .unwrap()
            .await
            .unwrap_err();
        assert!(matches!(error, CdpError::Protocol { code: -32000, .. }));

        client.close().unwrap();
        closed_rx.recv().unwrap();
        server.join().unwrap();
        assert!(client.is_closed());
        assert!(client.process_status().is_some());
        assert!(matches!(
            browser_events.recv().await,
            Err(EventError::Closed)
        ));
        assert_eq!(REMOTE_DEBUGGING_PIPE_ARG, "--remote-debugging-pipe");
    }

    #[tokio::test]
    async fn launcher_maps_chromes_fd3_and_fd4_and_owns_the_child() {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let capture = std::env::temp_dir().join(format!("cdp-launch-{unique}.json"));
        let expected = b"{\"id\":1,\"method\":\"Browser.getVersion\",\"params\":{}}\0";
        let script = r#"
            test "$0" = "--remote-debugging-pipe" || exit 91
            dd bs=1 count="$FRAME_LEN" <&3 >"$CAPTURE" 2>/dev/null || exit 92
            printf '{"id":1,"result":{"product":"FakeChrome"}}\0' >&4 || exit 93
            exec sleep 60
        "#;
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(script)
            .env("FRAME_LEN", expected.len().to_string())
            .env("CAPTURE", &capture);

        let client = CdpClient::launch_pipe(&mut command).unwrap();
        assert_eq!(
            client
                .send("Browser.getVersion", json!({}))
                .unwrap()
                .await
                .unwrap(),
            json!({"product": "FakeChrome"})
        );
        client.close().unwrap();
        assert!(client.process_status().is_some());
        assert_eq!(std::fs::read(&capture).unwrap(), expected);
        std::fs::remove_file(capture).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn launcher_spawn_error_closes_every_pipe_descriptor() {
        const ISOLATED_CHECK: &str = "BROWSER_CDP_FD_LEAK_CHECK";

        fn descriptor_count() -> usize {
            std::fs::read_dir("/proc/self/fd").unwrap().count()
        }

        if std::env::var_os(ISOLATED_CHECK).is_some() {
            let before = descriptor_count();
            let mut command = Command::new("/definitely/not/a/chrome/executable");
            let error = CdpClient::launch_pipe(&mut command).unwrap_err();
            assert!(matches!(error, CdpError::Transport(_)));
            assert_eq!(descriptor_count(), before);
            return;
        }

        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "unix::launcher_spawn_error_closes_every_pipe_descriptor",
            ])
            .env(ISOLATED_CHECK, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated descriptor check failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn websocket_routes_root_and_session_messages_and_closes() {
        use futures::{SinkExt, StreamExt};
        use tokio::net::TcpListener;
        use tokio_tungstenite::{accept_async, tungstenite::Message};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let root: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            let page: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(
                root,
                json!({"id": 1, "method": "Browser.getVersion", "params": {}})
            );
            assert_eq!(page["sessionId"], "page-ws");
            socket
                .send(Message::Text(
                    json!({"method": "Page.loadEventFired", "params": {}, "sessionId": "page-ws"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    json!({"id": 2, "result": {"frameTree": "ws"}, "sessionId": "page-ws"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    json!({"id": 1, "result": {"product": "WebSocketChrome"}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            matches!(socket.next().await, Some(Ok(Message::Close(_))))
        });

        let client =
            CdpClient::connect_websocket_url(&format!("ws://{address}/devtools/browser/fake"))
                .await
                .unwrap();
        let page = client.session("page-ws");
        let mut page_events = page.subscribe();
        let root_call = client.send("Browser.getVersion", json!({})).unwrap();
        let page_call = page.send("Page.getFrameTree", json!({})).unwrap();
        let (root, frame) = tokio::join!(root_call, page_call);
        assert_eq!(root.unwrap(), json!({"product": "WebSocketChrome"}));
        assert_eq!(frame.unwrap(), json!({"frameTree": "ws"}));
        assert_eq!(
            page_events.recv().await.unwrap().method,
            "Page.loadEventFired"
        );
        client.close().unwrap();
        assert!(server.await.unwrap());
    }

    #[tokio::test]
    async fn websocket_rejects_non_cdp_schemes_before_connecting() {
        let error = CdpClient::connect_websocket_url("http://127.0.0.1/devtools/browser/id")
            .await
            .unwrap_err();
        assert!(matches!(error, CdpError::UnsupportedTransport(_)));
        assert!(error.to_string().contains("ws:// or wss://"));

        let error = CdpClient::connect_websocket_url("wss://127.0.0.1:1/devtools/browser/id")
            .await
            .unwrap_err();
        assert!(matches!(error, CdpError::Transport(_)));
    }
}
