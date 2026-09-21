//! Bulk downloads use the same log directory as the browser listing.

use std::io::{Cursor, Read};

#[tokio::test]
async fn bulk_download_is_a_zip_of_regular_msl_files() {
    let temp = tempfile::tempdir().unwrap();
    let log_dir = temp.path().join("logs");
    let def = ts_ini::parse(rustytune_server::EMBEDDED_INI).unwrap();
    let state = rustytune_server::build_state(def, log_dir.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/api/logs/download.zip",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, rustytune_server::app(state))
            .await
            .unwrap();
    });
    let http = reqwest::Client::new();

    // A new installation with no log directory still produces a valid ZIP.
    let response = http.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let bytes = response.bytes().await.unwrap();
    assert!(zip::ZipArchive::new(Cursor::new(bytes)).unwrap().is_empty());

    std::fs::create_dir_all(&log_dir).unwrap();
    let logs = [
        ("first.msl", b"first log\n".as_slice()),
        ("second log.msl", b"second log\n".as_slice()),
    ];
    for (name, contents) in logs {
        std::fs::write(log_dir.join(name), contents).unwrap();
    }
    std::fs::write(log_dir.join("config.txt"), "not a log").unwrap();
    std::fs::create_dir(log_dir.join("directory.msl")).unwrap();
    #[cfg(unix)]
    {
        let secret = temp.path().join("secret.msl");
        std::fs::write(&secret, "outside log directory").unwrap();
        std::os::unix::fs::symlink(secret, log_dir.join("symlink.msl")).unwrap();
    }

    let response = http.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/zip");
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"rustytune-logs.zip\""
    );
    let bytes = response.bytes().await.unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    assert_eq!(archive.len(), logs.len());
    for (name, contents) in logs {
        let mut actual = Vec::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_end(&mut actual)
            .unwrap();
        assert_eq!(actual, contents);
    }
    server.abort();
}
