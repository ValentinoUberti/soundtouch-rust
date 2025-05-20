use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use bose_soundtouch::BoseClient;
use lazy_static::lazy_static;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::time::{timeout, Duration};
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    services::ServeDir,
};
use tracing::{error, info};
// Custom serializable Status struct
#[derive(Serialize, Deserialize)]
struct SerializableStatus {
    artist: String,
    track: String,
    volume: u32,
}

lazy_static! {
    static ref SELECTED_HOSTNAME: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
}

#[derive(Serialize, Deserialize, Clone)]
struct Device {
    hostname: String,
    ip: String,
    port: u16,
    realname: String,
}

#[derive(Serialize, Deserialize)]
struct StreamRequest {
    url: String,
}

async fn discover_devices() -> Vec<Device> {
    let mut devices = Vec::new();
    let mdns = match ServiceDaemon::new() {
        Ok(mdns) => mdns,
        Err(e) => {
            error!("Failed to create mDNS daemon: {}", e);
            return devices;
        }
    };
    let service_type = "_soundtouch._tcp.local.";
    let receiver = match mdns.browse(service_type) {
        Ok(receiver) => receiver,
        Err(e) => {
            error!("Failed to browse mDNS: {}", e);
            return devices;
        }
    };

    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        match timeout(Duration::from_millis(100), receiver.recv_async()).await {
            Ok(Ok(event)) => {
                if let ServiceEvent::ServiceResolved(info) = event {
                    let hostname = info.get_hostname().to_string();
                    let hostname_clone = hostname.clone();
                    let ip = info
                        .get_addresses()
                        .iter()
                        .next()
                        .map(|addr| addr.to_string())
                        .unwrap_or_default();
                    let port = info.get_port();
                    let realname= info.get_fullname().to_string();
                    devices.push(Device { hostname, ip, port, realname });
                    
                    let mut selected = SELECTED_HOSTNAME.lock().unwrap();
                    if selected.is_empty() {
                        *selected = hostname_clone.clone();
                        info!("Selected default device: {}", hostname_clone);
                    }
                }
            }
            Ok(Err(e)) => error!("mDNS receive error: {}", e),
            Err(_) => continue,
        }
    }

    if let Err(e) = mdns.shutdown() {
        error!("Failed to shutdown mDNS: {}", e);
    }
    info!("Discovered {} devices", devices.len());
    devices
}

async fn play_stream(hostname: &str, stream_url: &str) -> Result<(), StatusCode> {
    let client = Client::new();
    let xml_payload = format!(
        r#"<contentItem source="INTERNET_RADIO" location="{}" sourceAccount=""><itemName>Custom Stream</itemName></contentItem>"#,
        stream_url
    );

    let url = format!("http://{}:8090/select", hostname);
    let response = match client
        .post(&url)
        .body(xml_payload)
        .header("Content-Type", "application/xml")
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            error!("Failed to send stream request to {}: {}", url, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    if !response.status().is_success() {
        error!("Stream request failed with status: {}", response.status());
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    Ok(())
}

async fn play_pause(hostname: &str, action: &str) -> Result<(), StatusCode> {
    let client = Client::new();
    let xml_payload = format!(r#"<key state="press" sender="Gabbo">{}</key>"#, action);
    let url = format!("http://{}:8090/key", hostname);
    let response = match client
        .post(&url)
        .body(xml_payload)
        .header("Content-Type", "application/xml")
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            error!("Failed to send key request to {}: {}", url, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    if !response.status().is_success() {
        error!("Key request failed with status: {}", response.status());
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    Ok(())
}


async fn get_status() -> Result<Json<SerializableStatus>, StatusCode> {
    let hostname = SELECTED_HOSTNAME.lock().unwrap().clone();
    if hostname.is_empty() {
        info!("No device selected for status request");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    #[allow(deprecated)]
    let client = BoseClient::new(&hostname);
    
    let now_playing = match client.get_status().await {
        Ok(np) => np,
        Err(e) => {
            error!("Failed to get now playing from {}: {}", &hostname, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let volume = match client.get_volume().await {
        Ok(vol) => vol,
        Err(e) => {
            error!("Failed to get volume from {}: {}", &hostname, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    Ok(Json(SerializableStatus {
        artist: now_playing.artist.unwrap_or_default(),
        track: now_playing.track.unwrap_or_default(),
        volume: volume.actual.max(0) as u32,
    }))
}

async fn select_preset(Json(preset): Json<StreamRequest>) -> Result<(), StatusCode> {
    let hostname = SELECTED_HOSTNAME.lock().unwrap().clone();
    if hostname.is_empty() {
        info!("No device selected for preset request");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    #[allow(deprecated)]
    let client = BoseClient::new(&hostname);
    let preset_id: i32 = match preset.url.parse() {
        Ok(id) => id,
        Err(e) => {
            error!("Invalid preset ID {}: {}", preset.url, e);
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    match client.set_preset(preset_id).await {
        Ok(()) => Ok(()),
        Err(e) => {
            error!("Failed to set preset {} on {}: {}", preset.url, &hostname, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn set_volume(Json(volume): Json<StreamRequest>) -> Result<(), StatusCode> {
    let hostname = SELECTED_HOSTNAME.lock().unwrap().clone();
    if hostname.is_empty() {
        info!("No device selected for volume request");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let volume_value: u8 = match volume.url.parse() {
        Ok(value) => value,
        Err(e) => {
            error!("Invalid volume value {}: {}", volume.url, e);
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    #[allow(deprecated)]
    let client = BoseClient::new(&hostname);
    match client.set_volume(volume_value.into()).await {
        Ok(()) => Ok(()),
        Err(e) => {
            error!("Failed to set volume {} on {}: {}", volume_value, &hostname, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn play_radio(Json(request): Json<StreamRequest>) -> Result<(), StatusCode> {
    let hostname = SELECTED_HOSTNAME.lock().unwrap().clone();
    if hostname.is_empty() {
        info!("No device selected for radio request");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    play_stream(&hostname, &request.url).await?;
    Ok(())
}



async fn play_action(Json(action): Json<StreamRequest>) -> Result<(), StatusCode> {
    let hostname = SELECTED_HOSTNAME.lock().unwrap().clone();
    if hostname.is_empty() {
        info!("No device selected for play action");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    play_pause(&hostname, &action.url).await?;
    Ok(())
}

async fn discover() -> Result<Json<Vec<Device>>, StatusCode> {
    let devices = discover_devices().await;
    Ok(Json(devices))
}

async fn select_device(Json(device): Json<StreamRequest>) -> Result<(), StatusCode> {
    let mut selected = SELECTED_HOSTNAME.lock().unwrap();
    *selected = device.url.clone();
    info!("Device selected: {}", device.url);
    Ok(())
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    let hostname = SELECTED_HOSTNAME.lock().unwrap().clone();
    if hostname.is_empty() {
        let _ = socket
            .send(Message::Text(json!({ "error": "No device selected" }).to_string()))
            .await;
        return;
    }

    // Poll status periodically for WebSocket updates
    loop {
        let status = match get_status().await {
            Ok(status) => status.0,
            Err(_) => continue,
        };

        let message = json!({
            "type": "status",
            "artist": status.artist,
            "track": status.track,
            "volume": status.volume
        });

        if socket.send(Message::Text(message.to_string())).await.is_err() {
            break;
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn serve_index() -> impl IntoResponse {
    Html(include_str!("frontend/index.html"))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::exact("http://localhost:3000".parse().unwrap()))
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(serve_index))
        .route("/api/status", get(get_status))
        .route("/api/preset", post(select_preset))
        .route("/api/volume", post(set_volume))
        .route("/api/radio", post(play_radio))
        .route("/api/play", post(play_action))
        .route("/api/discover", get(discover))
        .route("/api/select_device", post(select_device))
        .route("/ws", get(ws_handler))
        .nest_service("/static", ServeDir::new("src/frontend"))
        .layer(cors);

    tokio::spawn(async {
        discover_devices().await;
    });

    info!("Starting server on http://0.0.0.0:3000");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}