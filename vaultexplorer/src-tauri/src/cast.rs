//! "Play on TV" over DIAL.
//!
//! Google's Cast SDK is a Play-services dependency with its own build
//! tooling, and it is not something this app can reasonably carry. DIAL
//! (DIscovery And Launch) is the open protocol underneath "open YouTube on
//! the TV": every Chromecast, Android TV and most smart TVs answer it, it
//! is two plain HTTP exchanges, and it needs no SDK at all.
//!
//! What it does *not* do is stream this device's screen -- it tells the TV
//! to open YouTube on the video itself, which is what "watch it on the TV"
//! means in practice and is how the YouTube app's own cast button behaves
//! for YouTube content.
//!
//! Discovery is an SSDP M-SEARCH: a UDP multicast asking who implements
//! the DIAL service, then a GET of each responder's description to learn
//! its name and its Application-URL (the header DIAL launches through).

use crate::errmap::ToStringErr;
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

#[derive(Serialize, Clone)]
pub struct CastDevice {
    pub name: String,
    /// The DIAL Application-URL, already normalised to end without a slash.
    pub app_url: String,
}

const SSDP_ADDR: &str = "239.255.255.250:1900";
const SEARCH: &str = "M-SEARCH * HTTP/1.1\r\n\
HOST: 239.255.255.250:1900\r\n\
MAN: \"ssdp:discover\"\r\n\
MX: 2\r\n\
ST: urn:dial-multiscreen-org:service:dial:1\r\n\r\n";

fn header_value(response: &str, name: &str) -> Option<String> {
    response.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
    })
}

/// Devices answering on the local network. Best-effort by nature: a TV
/// that is off, or a network that drops multicast, simply yields nothing,
/// which the UI shows as "no TVs found" rather than an error.
#[tauri::command]
pub async fn cast_discover() -> Result<Vec<CastDevice>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)).str_err()?;
        socket.set_read_timeout(Some(Duration::from_millis(600))).str_err()?;
        socket.send_to(SEARCH.as_bytes(), SSDP_ADDR).str_err()?;

        // Several replies arrive over a couple of seconds; collect until
        // the window closes rather than stopping at the first one.
        let mut locations: Vec<String> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut buf = [0u8; 2048];
        while Instant::now() < deadline {
            match socket.recv_from(&mut buf) {
                Ok((n, _)) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    if let Some(loc) = header_value(&text, "LOCATION") {
                        if !locations.contains(&loc) {
                            locations.push(loc);
                        }
                    }
                }
                Err(_) => continue, // read timeout: keep waiting out the window
            }
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .str_err()?;
        let mut devices = Vec::new();
        for loc in locations {
            let Ok(res) = client.get(&loc).send() else { continue };
            // The Application-URL is a *header* on this response, not part
            // of the XML body -- that is how DIAL specifies it.
            let app_url = res
                .headers()
                .get("Application-URL")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim_end_matches('/').to_string());
            let Some(app_url) = app_url else { continue };
            let body = res.text().unwrap_or_default();
            let name = body
                .split_once("<friendlyName>")
                .and_then(|(_, rest)| rest.split_once("</friendlyName>"))
                .map(|(n, _)| n.trim().to_string())
                .unwrap_or_else(|| "TV".to_string());
            if !devices.iter().any(|d: &CastDevice| d.app_url == app_url) {
                devices.push(CastDevice { name, app_url });
            }
        }
        Ok(devices)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Launches YouTube on the device and hands it the video. The body is the
/// form DIAL's YouTube app expects (`v=<id>`), so the TV opens that video
/// directly instead of the app's home screen.
#[tauri::command]
pub async fn cast_play_youtube(app_url: String, video_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(6))
            .build()
            .str_err()?;
        let res = client
            .post(format!("{}/YouTube", app_url.trim_end_matches('/')))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(format!("v={video_id}"))
            .send()
            .str_err()?;
        if !res.status().is_success() {
            return Err(format!("the TV refused the video (HTTP {})", res.status()));
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ssdp_headers_case_insensitively() {
        let res = "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=1800\r\nlocation: http://10.0.0.5:8008/ssdp/device-desc.xml\r\n\r\n";
        assert_eq!(
            header_value(res, "LOCATION").as_deref(),
            Some("http://10.0.0.5:8008/ssdp/device-desc.xml")
        );
        assert_eq!(header_value(res, "ST"), None);
    }
}
