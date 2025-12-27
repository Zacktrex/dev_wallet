// Display module for 0.42" 72x40 OLED screen (SSD1306)
// I2C interface on GPIO5 (SDA) and GPIO6 (SCL)
// Uses async I2C with ssd1306 0.10.0 async support

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Channel, Receiver, Sender};
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyleBuilder},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};
use ssd1306::{prelude::*, Ssd1306Async};
use log::info;
use esp_hal::i2c::master::I2c;

/// Channel for sending messages to display
pub type DisplayMessageChannel = Channel<CriticalSectionRawMutex, heapless::String<64>, 4>;
pub type DisplayMessageSender = Sender<'static, CriticalSectionRawMutex, heapless::String<64>, 4>;
pub type DisplayMessageReceiver = Receiver<'static, CriticalSectionRawMutex, heapless::String<64>, 4>;

/// Helper function to create a static display message channel
pub fn make_display_channel() -> &'static DisplayMessageChannel {
    use static_cell::StaticCell;
    static CHANNEL: StaticCell<DisplayMessageChannel> = StaticCell::new();
    CHANNEL.init(Channel::new())
}

/// Initialize the display channel and return sender/receiver
pub fn init_display_channel(
    channel: &'static DisplayMessageChannel,
) -> (DisplayMessageSender, DisplayMessageReceiver) {
    (channel.sender(), channel.receiver())
}

/// Display task that listens for messages and displays them on the 72x40 OLED
/// Note: Task functions cannot be generic, so we accept the concrete I2C type from esp-hal
/// The type matches: I2c::new(...).with_scl(...).with_sda(...).into_async()
#[embassy_executor::task]
pub async fn display_task(
    i2c: I2c<'static, esp_hal::Async>,
    receiver: DisplayMessageReceiver,
) {
    info!("Display task starting - initializing 72x40 OLED...");
    
    // Create I2C interface for SSD1306
    // SSD1306 default I2C address is 0x3C
    let interface = ssd1306::I2CDisplayInterface::new(i2c);
    
    // Initialize SSD1306 display with 72x40 resolution
    // Using DisplaySize72x40 for the 0.42" display
    let mut display = Ssd1306Async::new(
        interface,
        DisplaySize72x40,
        DisplayRotation::Rotate0,
    )
    .into_buffered_graphics_mode();
    
    // Initialize the display
    display.init().await.expect("Failed to initialize display");
    info!("Display initialized successfully (72x40 OLED)");
    
    // Create text style with 6x10 font (fits well on 72x40 display)
    let text_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(BinaryColor::On)
        .build();
    
    // Display startup message
    display.clear(BinaryColor::Off).unwrap();
    Text::with_baseline(
        "Dev Wallet",
        Point::new(0, 10),
        text_style,
        Baseline::Top,
    )
    .draw(&mut display)
    .unwrap();
    Text::with_baseline(
        "Ready",
        Point::new(0, 20),
        text_style,
        Baseline::Top,
    )
    .draw(&mut display)
    .unwrap();
    display.flush().await.expect("Failed to flush display");
    info!("Startup message displayed on OLED");
    
    // Main loop: listen for messages and display them
    loop {
        // Wait for messages from the channel
        let message = receiver.receive().await;
        info!("Received message for display: {}", message.as_str());
        
        // Clear the display
        display.clear(BinaryColor::Off).unwrap();
        
        // Split message into lines that fit the display (72 pixels wide, ~12 chars per line with 6x10 font)
        let msg_str = message.as_str();
        let max_chars_per_line = 12; // 72 pixels / 6 pixels per char = 12 chars
        
        // Break message into lines
        let mut y_pos = 10;
        let mut char_count = 0;
        let mut current_line = heapless::String::<12>::new();
        
        for ch in msg_str.chars() {
            if ch == '\n' || char_count >= max_chars_per_line {
                // Draw current line
                if !current_line.is_empty() {
                    Text::with_baseline(
                        current_line.as_str(),
                        Point::new(0, y_pos),
                        text_style,
                        Baseline::Top,
                    )
                    .draw(&mut display)
                    .unwrap();
                    y_pos += 13; // Line height
                    current_line.clear();
                    char_count = 0;
                    
                    if y_pos > 40 {
                        break; // Out of display bounds
                    }
                }
                
                if ch == '\n' {
                    continue;
                }
            }
            
            if current_line.push(ch).is_ok() {
                char_count += 1;
            } else {
                // Line buffer full, draw it and start new line
                Text::with_baseline(
                    current_line.as_str(),
                    Point::new(0, y_pos),
                    text_style,
                    Baseline::Top,
                )
                .draw(&mut display)
                .unwrap();
                y_pos += 13;
                current_line.clear();
                if current_line.push(ch).is_ok() {
                    char_count = 1;
                }
                
                if y_pos > 40 {
                    break;
                }
            }
        }
        
        // Draw remaining line if any
        if !current_line.is_empty() && y_pos <= 40 {
            Text::with_baseline(
                current_line.as_str(),
                Point::new(0, y_pos),
                text_style,
                Baseline::Top,
            )
            .draw(&mut display)
            .unwrap();
        }
        
        // Flush the display buffer to show the message
        display.flush().await.expect("Failed to flush display");
        info!("Message displayed on OLED successfully");
    }
}
