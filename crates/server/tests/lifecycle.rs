use std::time::Duration;

#[tokio::test]
async fn shared_server_lifecycle_serves_and_stops() {
    let def = ts_ini::parse(rustytune_server::EMBEDDED_INI).unwrap();
    let state = rustytune_server::build_state(def, std::env::temp_dir());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let server_state = state.clone();
    let server = tokio::spawn(async move {
        rustytune_server::serve_with_shutdown(listener, server_state, async move {
            let _ = shutdown_rx.await;
        })
        .await
    });

    let response = reqwest::get(format!("http://{addr}/api/health"))
        .await
        .unwrap();
    assert!(response.status().is_success());

    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("server did not stop")
        .expect("server task panicked")
        .expect("server returned an error");

    // Cleanup is deliberately idempotent for overlapping desktop exit events.
    rustytune_server::shutdown_comms(&state);
}
