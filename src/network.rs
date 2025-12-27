// Network stack setup and management

use embassy_net::{Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_hal::rng::Rng;
use esp_radio::wifi::WifiDevice;
use log::info;
use core::net::Ipv4Addr;
use core::str::FromStr;
use crate::config::{STATIC_IP, GATEWAY_IP};

/// Initialize and configure the network stack
pub fn setup_network_stack(
    wifi_interface: WifiDevice<'static>,
) -> (Stack<'static>, Runner<'static, WifiDevice<'static>>) {
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
    
    // Create network stack with resources
    use static_cell::StaticCell;
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
    
    (stack, runner)
}

/// Wait for network link to be established
pub async fn wait_for_network_link(stack: &Stack<'static>) {
    info!("Waiting for network link...");
    loop {
        if stack.is_link_up() {
            info!("Network link is UP");
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}

/// Wait for IP configuration
pub async fn wait_for_ip_config(stack: &Stack<'static>) {
    info!("Waiting for IP configuration...");
    loop {
        if let Some(config) = stack.config_v4() {
            info!("Network configured! IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}

/// Network stack runner task
#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    info!("Network stack task started");
    runner.run().await
}

