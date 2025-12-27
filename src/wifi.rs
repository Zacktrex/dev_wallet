// Wi-Fi Access Point setup and monitoring

use embassy_time::{Duration, Timer};
use esp_radio::wifi::{AccessPointConfig, ModeConfig, WifiController, WifiEvent, WifiApState};
use log::info;
use crate::config::{SSID, PASSWORD};

/// Configure and start the Wi-Fi Access Point
pub async fn setup_access_point(
    mut controller: WifiController<'static>,
) -> WifiController<'static> {
    info!("Configuring Wi-Fi Access Point...");
    info!("SSID: {}", SSID);
    info!("SSID Broadcasting: enabled (not hidden)");
    
    let ap_config = ModeConfig::AccessPoint(
        AccessPointConfig::default()
            .with_ssid(SSID.into())
            .with_password(PASSWORD.into())
            .with_auth_method(esp_radio::wifi::AuthMethod::Wpa2Personal)
            .with_ssid_hidden(false),
    );
    
    controller.set_config(&ap_config).unwrap();
    info!("Starting Wi-Fi Access Point...");
    controller.start_async().await.unwrap();
    info!("Wi-Fi Access Point started successfully!");
    
    controller
}

/// Monitor Wi-Fi AP state and restart if needed
#[embassy_executor::task]
pub async fn wifi_monitor_task(mut controller: WifiController<'static>) {
    info!("Wi-Fi monitor task started");
    
    loop {
        match esp_radio::wifi::ap_state() {
            WifiApState::Started => {
                info!("Wi-Fi AP is running and advertising");
                // Wait for stop event
                controller.wait_for_event(WifiEvent::ApStop).await;
                info!("Wi-Fi AP stopped - will restart");
                Timer::after(Duration::from_millis(5000)).await;
                
                // Restart AP
                let ap_config = ModeConfig::AccessPoint(
                    AccessPointConfig::default()
                        .with_ssid(SSID.into())
                        .with_password(PASSWORD.into())
                        .with_auth_method(esp_radio::wifi::AuthMethod::Wpa2Personal)
                        .with_ssid_hidden(false),
                );
                controller.set_config(&ap_config).unwrap();
                controller.start_async().await.unwrap();
                info!("Wi-Fi AP restarted and advertising");
            }
            _ => {
                if !matches!(controller.is_started(), Ok(true)) {
                    info!("Wi-Fi AP not started, attempting to start...");
                    let ap_config = ModeConfig::AccessPoint(
                        AccessPointConfig::default()
                            .with_ssid(SSID.into())
                            .with_password(PASSWORD.into())
                            .with_auth_method(esp_radio::wifi::AuthMethod::Wpa2Personal)
                            .with_ssid_hidden(false),
                    );
                    controller.set_config(&ap_config).unwrap();
                    controller.start_async().await.unwrap();
                    info!("Wi-Fi AP started and advertising");
                }
                Timer::after(Duration::from_secs(5)).await;
            }
        }
    }
}

