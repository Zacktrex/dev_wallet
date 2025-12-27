#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use bt_hci::controller::ExternalController;
use embassy_executor::Spawner;
use embassy_net::{Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use esp_radio::wifi::{AccessPointConfig, ModeConfig, WifiDevice};
use log::info;
use trouble_host::prelude::*;
use core::net::Ipv4Addr;
use core::str::FromStr;
use picoserve::{response::File, routing::get_service, AppBuilder, Router};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

extern crate alloc;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 1;

// Wi-Fi Access Point configuration
const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");
// IP configuration for the AP
const STATIC_IP: &str = "192.168.4.1/24";
const GATEWAY_IP: &str = "192.168.4.1";

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.0.1

    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320);
    // COEX needs more RAM - so we've added some more
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    info!("Embassy initialized!");

    // Make radio_init static so it lives long enough for both Wi-Fi and BLE
    use static_cell::StaticCell;
    static RADIO_INIT: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
    let radio_init = RADIO_INIT.init(esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller"));
    
    // Initialize and configure Wi-Fi Access Point
    let (mut wifi_controller, interfaces) =
        esp_radio::wifi::new(radio_init, peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");
    
    info!("Configuring Wi-Fi Access Point...");
    info!("SSID: {}", SSID);
    info!("SSID Broadcasting: enabled (not hidden)");
    
    let ap_config = ModeConfig::AccessPoint(
        AccessPointConfig::default()
            .with_ssid(SSID.into())
            .with_password(PASSWORD.into())
            .with_auth_method(esp_radio::wifi::AuthMethod::Wpa2Personal)
            .with_ssid_hidden(false), // Enable SSID broadcasting
    );
    
    wifi_controller.set_config(&ap_config).unwrap();
    info!("Starting Wi-Fi Access Point...");
    wifi_controller.start_async().await.unwrap();
    info!("Wi-Fi Access Point started successfully!");
    
    // Initialize network stack for the AP
    let wifi_interface = interfaces.ap;
    let rng = Rng::new();
    let net_seed = rng.random() as u64 | ((rng.random() as u64) << 32);
    
    let Ok(ip_addr) = Ipv4Cidr::from_str(STATIC_IP) else {
        info!("Invalid STATIC_IP: {}", STATIC_IP);
        loop {}
    };
    
    let Ok(gateway) = Ipv4Addr::from_str(GATEWAY_IP) else {
        info!("Invalid GATEWAY_IP: {}", GATEWAY_IP);
        loop {}
    };
    
    let net_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: ip_addr,
        gateway: Some(gateway),
        dns_servers: Default::default(),
    });
    
    // Create network stack with more resources for better connection handling
    static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
    let stack_resources = STACK_RESOURCES.init(StackResources::<4>::new());
    
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        net_config,
        stack_resources,
        net_seed,
    );
    
    info!("Network stack initialized");
    info!("AP IP Address: {}", ip_addr);
    info!("AP Gateway: {}", gateway);
    
    // Spawn network stack runner task (required for network to work)
    spawner.spawn(net_task(runner)).ok();
    info!("Network stack task spawned");
    
    // Wait for network link to be up
    info!("Waiting for network link...");
    loop {
        if stack.is_link_up() {
            info!("Network link is UP");
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
    
    // Wait for IP configuration
    info!("Waiting for IP configuration...");
    loop {
        if let Some(config) = stack.config_v4() {
            info!("Network configured! IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
    
    info!("AP is now advertising and ready for connections");
    info!("ESP32's built-in DHCP server should assign IPs to clients");
    info!("Clients should get IPs in range: 192.168.4.2 - 192.168.4.254");
    
    info!("AP is ready - devices can now connect");
    
    // Set up web server
    info!("Setting up web server...");
    let router = make_static_router();
    let config = make_static_config();
    
    // Spawn web server task (stack is moved into the task)
    spawner.spawn(web_server_task(0, stack, router, config)).ok();
    info!("Web server started on http://192.168.4.1");
    
    // Spawn task to monitor Wi-Fi AP
    spawner.spawn(wifi_monitor_task(wifi_controller)).ok();
    
    // find more examples https://github.com/embassy-rs/trouble/tree/main/examples/esp32
    let transport = BleConnector::new(radio_init, peripherals.BT, Default::default()).unwrap();
    let ble_controller = ExternalController::<_, 20>::new(transport);
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let _stack = trouble_host::new(ble_controller, &mut resources);

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples/src/bin
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    info!("Network stack task started");
    runner.run().await
}

struct WebApp;

impl AppBuilder for WebApp {
    type PathRouter = impl picoserve::routing::PathRouter;

    fn build_app(self) -> Router<Self::PathRouter> {
        Router::new().route(
            "/",
            get_service(File::html(include_str!("../index.html"))),
        )
    }
}

fn make_static_router() -> &'static picoserve::AppRouter<WebApp> {
    use static_cell::StaticCell;
    static ROUTER: StaticCell<picoserve::AppRouter<WebApp>> = StaticCell::new();
    ROUTER.init(WebApp.build_app())
}

fn make_static_config() -> &'static picoserve::Config<Duration> {
    use static_cell::StaticCell;
    static CONFIG: StaticCell<picoserve::Config<Duration>> = StaticCell::new();
    CONFIG.init(picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
        persistent_start_read_request: Some(Duration::from_secs(1)),
    })
    .keep_connection_alive())
}

#[embassy_executor::task]
async fn web_server_task(
    task_id: usize,
    stack: Stack<'static>,
    router: &'static picoserve::AppRouter<WebApp>,
    config: &'static picoserve::Config<Duration>,
) -> ! {
    let port = 80;
    let mut tcp_rx_buffer = [0; 1024];
    let mut tcp_tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    info!("Web server task {} starting on port {}", task_id, port);
    info!("Web server ready to accept connections on http://192.168.4.1");
    
    // Monitor for device connections by checking link state
    let mut last_link_state = stack.is_link_up();
    let mut connection_logged = false;
    
    if last_link_state {
        info!("Network link is up - device may already be connected");
        info!("🔌 Device connected to Wi-Fi AP!");
        connection_logged = true;
    } else {
        info!("Waiting for device to connect...");
    }
    
    // Monitor link state changes to detect connections before starting server
    // After server starts, connection detection continues via network activity
    loop {
        let current_link_state = stack.is_link_up();
        
        if current_link_state != last_link_state {
            if current_link_state && !connection_logged {
                info!("🔌 Device connected to Wi-Fi AP!");
                
                // Wait a moment for IP assignment
                Timer::after(Duration::from_millis(2000)).await;
                
                if let Some(net_config) = stack.config_v4() {
                    info!("Network configured. AP IP: {}", net_config.address);
                }
                info!("Connected device can now access the web server at http://192.168.4.1");
                connection_logged = true;
            } else if !current_link_state && connection_logged {
                info!("🔌 Device disconnected from Wi-Fi AP");
                connection_logged = false;
            }
            last_link_state = current_link_state;
        }
        
        // Start server once link is up (or immediately if already up)
        if current_link_state {
            break;
        }
        
        Timer::after(Duration::from_millis(500)).await;
    }
    
    info!("Starting web server - ready to serve requests");
    picoserve::Server::new(router, config, &mut http_buffer)
        .listen_and_serve(task_id, stack, port, &mut tcp_rx_buffer, &mut tcp_tx_buffer)
        .await
        .into_never()
}

#[embassy_executor::task]
async fn wifi_monitor_task(mut controller: esp_radio::wifi::WifiController<'static>) {
    use esp_radio::wifi::{WifiApState, WifiEvent};
    
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

