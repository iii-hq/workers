use super::*;

#[tokio::test]
async fn sdk_current_thread_dispatch_keeps_timers_running_during_storage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store.sqlite3");
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let task = tokio::spawn(crate::webhooks::wiring::storage_task(async move {
        let store = Store::open(&path)?;
        store.change(|_| {
            started_tx.send(()).unwrap();
            finish_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            Ok(())
        })
    }));
    started_rx.await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    finish_tx.send(()).unwrap();
    task.await.unwrap().unwrap();
}
