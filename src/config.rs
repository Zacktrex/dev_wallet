// Configuration constants for the ESP32-C3 Dev Wallet

// Wi-Fi Access Point configuration
pub const SSID: &str = env!("SSID");
pub const PASSWORD: &str = env!("PASSWORD");

// IP configuration for the AP
pub const STATIC_IP: &str = "192.168.4.1/24";
pub const GATEWAY_IP: &str = "192.168.4.1";

// BLE configuration
pub const CONNECTIONS_MAX: usize = 1;
pub const L2CAP_CHANNELS_MAX: usize = 1;

// Web server configuration
pub const WEB_SERVER_PORT: u16 = 80;

