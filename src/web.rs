// Web server, routes, and HTTP handlers

use embassy_net::Stack;
use embassy_time::Duration;
use log::info;
use picoserve::{response::File, routing::get_service, AppBuilder, Router};
use picoserve::routing::PathRouter;
use crate::config::WEB_SERVER_PORT;
use crate::display::DisplayMessageSender;

extern crate alloc;

/// Static storage for display sender (set by make_static_router_with_display)
static DISPLAY_SENDER: static_cell::StaticCell<Option<DisplayMessageSender>> = static_cell::StaticCell::new();
static mut DISPLAY_SENDER_REF: Option<&'static DisplayMessageSender> = None;

/// Set the display sender (called by make_static_router_with_display)
pub fn set_display_sender(sender: DisplayMessageSender) {
    unsafe {
        let sender_ref = DISPLAY_SENDER.init(Some(sender));
        if let Some(s) = sender_ref.as_ref() {
            DISPLAY_SENDER_REF = Some(s);
        }
    }
}

/// Send message to display via the channel
fn send_to_display(message: &str) {
    unsafe {
        if let Some(sender) = DISPLAY_SENDER_REF {
            use heapless::String;
            let mut display_msg = String::<64>::new();
            if display_msg.push_str(message).is_ok() {
                let _ = sender.try_send(display_msg);
            }
        }
    }
}

/// Custom service that logs query parameters from form submissions
pub struct LoggingSubmitService;

impl<State, PathParameters> picoserve::routing::RequestHandlerService<State, PathParameters> for LoggingSubmitService {
    async fn call_request_handler_service<R: picoserve::io::Read, W: picoserve::response::ResponseWriter<Error = R::Error>>(
        &self,
        _state: &State,
        _path_parameters: PathParameters,
        request: picoserve::request::Request<'_, R>,
        response_writer: W,
    ) -> Result<picoserve::ResponseSent, W::Error> {
        // Extract message from query string (format: /submit?message=value)
        if let Some(query) = request.parts.query() {
            // UrlEncodedString is a tuple struct, access the underlying &str with .0
            let query_str = query.0;
            
            if let Some(msg_start) = query_str.find("message=") {
                let value_start = msg_start + 8; // "message=".len()
                let value = &query_str[value_start..];

                // URL decode: replace + with space and %20 with space
                let mut decoded = alloc::string::String::new();
                let mut chars = value.chars();
                while let Some(ch) = chars.next() {
                    match ch {
                        '+' => decoded.push(' '),
                        '%' => {
                            // Simple URL decode for %20
                            if let (Some('2'), Some('0')) = (chars.next(), chars.next()) {
                                decoded.push(' ');
                            } else {
                                decoded.push(ch);
                            }
                        }
                        '&' => break, // Stop at next parameter
                        _ => decoded.push(ch),
                    }
                }

                // Log to console - THIS WILL PRINT IN DEBUG TERMINAL
                let message = decoded.trim();
                info!("📨 Received message from web form: {}", message);
                
                // Send message to display via the channel
                // The sender is stored statically and accessed here
                send_to_display(message);
            }
        }

        // Return the HTML page with proper Content-Type header
        let html_content = include_str!("index.html");
        let response = picoserve::response::Response::new(
            picoserve::response::status::StatusCode::OK,
            html_content,
        )
        .with_header("Content-Type", "text/html; charset=utf-8");
        
        // Convert RequestBodyConnection to Connection using finalize()
        let connection = request.body_connection.finalize().await?;
        response_writer.write_response(connection, response).await
    }
}

/// Web application structure
pub struct WebApp;

impl AppBuilder for WebApp {
    type PathRouter = impl PathRouter;

    fn build_app(self) -> Router<Self::PathRouter> {
        Router::new()
            .route(
                "/",
                get_service(File::html(include_str!("index.html"))),
            )
            .route(
                "/submit",
                get_service(LoggingSubmitService),
            )
    }
}

/// Create router with display sender
/// The sender is stored and messages will be sent to the display task
pub fn make_static_router_with_display(display_sender: DisplayMessageSender) -> &'static picoserve::AppRouter<WebApp> {
    set_display_sender(display_sender);
    make_static_router()
}


/// Create a static router instance
pub fn make_static_router() -> &'static picoserve::AppRouter<WebApp> {
    use static_cell::StaticCell;
    static ROUTER: StaticCell<picoserve::AppRouter<WebApp>> = StaticCell::new();
    ROUTER.init(WebApp.build_app())
}

/// Create a static server configuration
pub fn make_static_config() -> &'static picoserve::Config<Duration> {
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

/// Web server task that handles HTTP requests
#[embassy_executor::task]
pub async fn web_server_task(
    task_id: usize,
    stack: Stack<'static>,
    router: &'static picoserve::AppRouter<WebApp>,
    config: &'static picoserve::Config<Duration>,
) -> ! {
    use embassy_time::{Duration, Timer};
    
    let mut tcp_rx_buffer = [0; 1024];
    let mut tcp_tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    info!("Web server task {} starting on port {}", task_id, WEB_SERVER_PORT);
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
        .listen_and_serve(task_id, stack, WEB_SERVER_PORT, &mut tcp_rx_buffer, &mut tcp_tx_buffer)
        .await
        .into_never()
}

