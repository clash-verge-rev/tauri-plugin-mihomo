use std::time::Duration;

use tauri_plugin_mihomo::{Mihomo, models::Protocol};

#[allow(dead_code)]
pub const TEST_URL: &str = "http://www.gstatic.com/generate_204";
#[allow(dead_code)]
pub const TIMEOUT: u32 = 3000;

pub fn mihomo() -> Mihomo {
    #[allow(clippy::unwrap_used)]
    dotenvy::dotenv().unwrap();
    let mihomo_socket = std::env::var("MIHOMO_SOCKET").unwrap_or(String::from("0"));
    if mihomo_socket == "1" {
        println!("connect to mihomo by local socket");
        // use local socket
        let socket_path = if cfg!(unix) {
            "/tmp/verge-mihomo.sock".to_string()
            // "/tmp/clash-rs.sock".to_string()
        } else {
            r"\\.\pipe\verge-mihomo".to_string()
            // r"\\.\pipe\clash-rs".to_string()
        };
        #[allow(clippy::unwrap_used)]
        Mihomo::new(
            Protocol::LocalSocket,
            None,
            None,
            None,
            Some(socket_path),
            Duration::from_millis(100),
        )
        .unwrap()
    } else {
        println!("connect to mihomo by http");
        // use http
        #[allow(clippy::unwrap_used)]
        Mihomo::new(
            Protocol::Http,
            Some("127.0.0.1".into()),
            Some(9090),
            Some("yPMJk9i7UaR1hv3-2BkPy".into()),
            None,
            Duration::from_secs(1),
        )
        .unwrap()
    }
}
