use simple_http_server::server::{Server, ServerConfig};

#[tokio::main]
async fn main() {
    println!("A Simple Http Server");
    let mut server = Server::new(
        "0.0.0.0:8088",
        ServerConfig {
            max_header_size: 8192,
            max_body_size: 10 * 1024 * 1024,
        },
    );
    if let Err(e) = server.serve().await {
        eprintln!("server error: {:?}", e);
    }
}
