//! Main entry point for the Yello server in Rust.

use hyper::service::{make_service_fn, service_fn};
use hyper::Server;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use yello_server::server::{handle_device_ws, handle_http_request, start_background_tasks, AppState};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let web_host = std::env::var("YELLO_WEB_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let web_port: u16 = std::env::var("YELLO_WEB_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
    let device_host = std::env::var("YELLO_DEVICE_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let device_ws_port: u16 = std::env::var("YELLO_DEVICE_WS_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8765);

    let studio_path = std::env::var("YELLO_STUDIO_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("studio.json"));

    let secrets_path = std::env::var("YELLO_SECRETS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("secrets.json"));

    let static_dir = std::env::var("YELLO_STATIC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            if PathBuf::from("server-rs/static").exists() {
                PathBuf::from("server-rs/static")
            } else {
                PathBuf::from("static")
            }
        });

    println!("Starting Yello Rust Server...");
    println!("Web studio: http://{}:{}", web_host, web_port);
    println!("Device WebSocket: ws://{}:{}", device_host, device_ws_port);
    println!("Studio file: {:?}", studio_path);
    println!("Static directory: {:?}", static_dir);

    let state = Arc::new(AppState::new(studio_path, secrets_path, static_dir));
    state.load_settings().await;
    state.load_studio_file().await;

    start_background_tasks(state.clone());

    // Start WebSocket TCP Listener
    let ws_addr: SocketAddr = format!("{}:{}", device_host, device_ws_port).parse()?;
    let ws_listener = tokio::net::TcpListener::bind(ws_addr).await?;
    println!("WebSocket server listening on {}", ws_addr);

    let ws_state = state.clone();
    tokio::spawn(async move {
        while let Ok((stream, addr)) = ws_listener.accept().await {
            let st = ws_state.clone();
            tokio::spawn(async move {
                handle_device_ws(stream, addr, st).await;
            });
        }
    });

    // Start HTTP Server
    let http_addr: SocketAddr = format!("{}:{}", web_host, web_port).parse()?;
    let http_state = state.clone();
    let make_svc = make_service_fn(move |_conn| {
        let st = http_state.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let s = st.clone();
                async move { handle_http_request(req, s).await }
            }))
        }
    });

    let http_server = Server::bind(&http_addr).serve(make_svc);
    println!("HTTP server listening on http://{}", http_addr);

    http_server.await?;
    Ok(())
}
