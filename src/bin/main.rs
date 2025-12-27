#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use bt_hci::controller::ExternalController;
use dev_wallet::{config, display, network, web, wifi};
use embassy_executor::Spawner;
use embassy_time::Timer;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use log::info;
use trouble_host::prelude::*;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    info!("Embassy initialized!");

    // Initialize radio for Wi-Fi and BLE
    use static_cell::StaticCell;
    static RADIO_INIT: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
    let radio_init =
        RADIO_INIT.init(esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller"));

    // Initialize and configure Wi-Fi Access Point
    let (mut wifi_controller, interfaces) =
        esp_radio::wifi::new(radio_init, peripherals.WIFI, Default::default())
            .expect("Failed to initialize Wi-Fi controller");

    wifi_controller = wifi::setup_access_point(wifi_controller).await;

    // Initialize network stack
    let wifi_interface = interfaces.ap;
    let (stack, runner) = network::setup_network_stack(wifi_interface);

    // Spawn network stack runner task
    spawner.spawn(network::net_task(runner)).ok();
    info!("Network stack task spawned");

    // Wait for network to be ready
    network::wait_for_network_link(&stack).await;
    network::wait_for_ip_config(&stack).await;

    info!("AP is now advertising and ready for connections");
    info!("ESP32's built-in DHCP server should assign IPs to clients");
    info!("Clients should get IPs in range: 192.168.4.2 - 192.168.4.254");
    info!("AP is ready - devices can now connect");

    // Initialize I2C for display (GPIO5 = SDA, GPIO6 = SCL)
    use esp_hal::i2c::master::{Config as I2cConfig, I2c};
    use esp_hal::time::Rate;

    let i2c_bus = I2c::new(
        peripherals.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(400)),
    )
    .unwrap()
    .with_scl(peripherals.GPIO6) // SCL on GPIO6
    .with_sda(peripherals.GPIO5) // SDA on GPIO5
    .into_async();

    info!("I2C initialized for display (GPIO5=SDA, GPIO6=SCL @ 400kHz)");

    // Set up web server with display support
    info!("Setting up web server...");
    let display_channel = display::make_display_channel();
    let (display_sender, display_receiver) = display::init_display_channel(display_channel);

    // Spawn display task with I2C interface
    spawner
        .spawn(display::display_task(i2c_bus, display_receiver))
        .ok();
    info!("Display task spawned (72x40 OLED on I2C)");

    // Create router with display sender to show messages on screen
    let router = web::make_static_router_with_display(display_sender);
    let web_config = web::make_static_config();

    spawner
        .spawn(web::web_server_task(0, stack, router, web_config))
        .ok();
    info!("Web server started on http://192.168.4.1");

    // Spawn task to monitor Wi-Fi AP
    spawner.spawn(wifi::wifi_monitor_task(wifi_controller)).ok();

    // Initialize BLE stack
    let transport = BleConnector::new(radio_init, peripherals.BT, Default::default()).unwrap();
    let ble_controller = ExternalController::<_, 20>::new(transport);
    let mut resources: HostResources<
        DefaultPacketPool,
        { config::CONNECTIONS_MAX },
        { config::L2CAP_CHANNELS_MAX },
    > = HostResources::new();
    let _stack = trouble_host::new(ble_controller, &mut resources);

    loop {
        Timer::after(embassy_time::Duration::from_secs(1)).await;
    }
}
