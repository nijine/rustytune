# RustyTune Tauri Desktop Application

## Summary

Add a Tauri 2 desktop shell that runs the existing Axum server and ECU communication in-process, then displays the embedded React UI in a native WebView. Retain the current CLI/browser and Raspberry Pi appliance workflows unchanged.

Version 1 targets macOS arm64 and Linux x86_64. It exits completely when its window closes and adds no native file-dialog rewrite.

## Implementation Changes

### Desktop application

- Add a `rustytune-desktop` workspace crate with:
  - Product name `RustyTune`
  - Bundle identifier `com.github.nijine.rustytune`
  - One resizable 1200×800 window with an 800×600 minimum
  - Tauri 2, the single-instance plugin, and no JavaScript-accessible Tauri commands
- On startup:
  1. Resolve Tauri’s per-user application-data directory and create its `logs` subdirectory.
  2. Build the normal desktop runtime configuration with browser opening disabled and that absolute log directory.
  3. Parse the embedded Speeduino INI and construct the existing server state.
  4. Bind Axum to `127.0.0.1:0` so the OS selects an unused port.
  5. Start Axum and open the WebView at the resulting loopback URL.
- Permit navigation only within the selected loopback origin. Open HTTPS help links in the OS browser and reject other external navigation.
- A second launch should focus and restore the existing window instead of starting another server or competing for the serial port.
- If initialization fails, report the error to stderr, terminate with a nonzero status, and ensure no partial ECU/server process remains.
- Add a dark rounded-square “RT” icon using the existing `#3987e5` accent, white lettering, and `#0d0d0d` background. Keep the source SVG and generated Tauri PNG/ICNS assets in the desktop crate.

### Shared server lifecycle

- Move reusable serving and cleanup behavior from the CLI entry point into `rustytune-server`:
  - A public async server function accepting a pre-bound `tokio::net::TcpListener`, shared state, and shutdown future.
  - A single cleanup path that sends `Cmd::Shutdown`, joins the communication thread, flushes active datalogs, and closes the serial device.
- Update the existing CLI binary to call this shared lifecycle function while preserving every current flag, signal, `q`-to-quit behavior, browser launch, and appliance behavior.
- Have the Tauri close handler prevent immediate process exit, destroy/hide the WebView so REST/WebSocket connections close, signal server shutdown, await communication cleanup, and then exit. Apply a bounded shutdown timeout with an error log before forced termination.
- If the internal server exits unexpectedly while the window is open, close the application with a nonzero status.
- Keep the existing relative REST URLs, WebSocket URL construction, embedded assets, authentication behavior, and API wire formats unchanged.

### Build and release

- Add `make desktop-run` and `make desktop-build` targets. Both build `web/dist` first so the current source SHA and frontend assets are embedded before compiling the desktop crate.
- Document Tauri/Linux development prerequisites and the new commands in the README.
- Update regular CI to install WebKitGTK 4.1 and related Tauri packages on Linux before workspace checks. Continue running frontend build, formatting, Clippy, and all Rust tests on macOS and Linux.
- Extend tagged draft releases without removing current CLI tarballs:
  - macOS arm64: build on an arm64 macOS runner and upload a DMG.
  - Linux x86_64: build the AppImage on Ubuntu 22.04 for a conservative glibc/WebKitGTK baseline.
  - Keep Linux arm64 as the existing headless CLI/appliance artifact only.
- Preserve `VITE_BUILD_SHA` in desktop builds and use the workspace version as the Tauri bundle version.
- Make macOS signing optional through an explicit `MACOS_SIGNING_ENABLED` repository variable. When enabled, require Developer ID certificate, password, identity, Apple ID/app password, and team ID secrets and fail the release job if signing or notarization fails. When disabled, produce an unsigned draft artifact and label it accordingly. Follow Tauri’s [macOS signing guidance](https://v2.tauri.app/distribute/sign/macos/) and [AppImage baseline guidance](https://v2.tauri.app/distribute/appimage/).

## Public Interfaces

- Add only the reusable server lifecycle functions described above; introduce no REST, WebSocket, JSON, frontend, or configuration-schema changes.
- The desktop executable accepts no CLI configuration flags in v1. Advanced configuration continues through the existing `rustytune` CLI.
- Desktop datalogs live beneath Tauri’s platform-specific application-data directory rather than depending on the GUI process’s working directory.

## Test Plan

- Unit/integration test the shared lifecycle using an ephemeral listener:
  - `/api/health` becomes reachable.
  - Shutdown resolves cleanly.
  - Communication cleanup runs once even after a server error.
- Run all existing server end-to-end tests unchanged to prove the refactor preserves API and ECU behavior.
- Verify CI can compile and bundle both desktop targets from a clean checkout.
- Manually validate installed macOS and Linux bundles:
  - Launching opens one native window and no browser.
  - REST calls and telemetry WebSockets use the internal ephemeral server.
  - Fake-ECU connection, tuning, burns, datalogging, offline MSQ editing, imports, and downloads still work.
  - HTTPS help opens externally without navigating the main window.
  - A second launch focuses the existing instance.
  - Closing during an ECU connection or active log flushes the log, closes the serial device, releases the port, and leaves no server process.
  - Launching from Finder or the Linux desktop uses the application-data log directory correctly.
- Exercise both unsigned and signing-enabled release paths; signing-enabled builds must pass notarization before upload.

## Assumptions

- Tauri 2 is used and dependency resolution is committed through `Cargo.lock`.
- Windows remains out of scope until the Unix-only serial transport receives a Windows backend.
- Browser upload/download controls remain the v1 file workflow; native dialogs, menus, tray behavior, auto-update, and window-state persistence are deferred.
- The existing CLI and appliance artifacts remain supported and are released alongside the new desktop bundles.
