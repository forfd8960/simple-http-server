use simple_http_server::server::Server;

#[tokio::main]
async fn main() {
    println!("A Simple Http Server");
    let mut server = Server::new("0.0.0.0:8088");
    if let Err(e) = server.serve().await {
        eprintln!("server error: {:?}", e);
    }
}
