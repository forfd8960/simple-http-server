use std::sync::Arc;

use simple_http_server::{
    req_res::{HttpRequest, HttpResponse},
    router::Router,
    server::{Server, ServerConfig},
};

#[tokio::main]
async fn main() {
    println!("A Simple Http Server");
    let mut server = Server::new(
        "127.0.0.1:8088",
        ServerConfig {
            max_header_size: 8192,
            max_body_size: 10 * 1024 * 1024,
        },
    );

    let mut router = Router::new();

    // Static routes
    router.get("/index", |_req| async {
        HttpResponse::new(200, None, Some(b"Home".to_vec()))
    });

    router.get("/hello", hello);

    // Route with single parameter
    router.get("/api/users/:id", |req| async move {
        let user_id = req.param("id").unwrap_or("unknown");
        let body = format!("User ID: {}", user_id);
        HttpResponse::new(200, None, Some(body.into_bytes()))
    });

    // Route with multiple parameters
    router.get("/api/users/:user_id/posts/:post_id", |req| async move {
        let user_id = req.param("user_id").unwrap_or("?");
        let post_id = req.param("post_id").unwrap_or("?");

        let body = format!("User: {}, Post: {}", user_id, post_id);
        HttpResponse::new(200, None, Some(body.into_bytes()))
    });

    let r = Arc::new(router);
    if let Err(e) = server.serve(r).await {
        eprintln!("server error: {:?}", e);
    }
}

async fn hello(req: HttpRequest) -> HttpResponse {
    println!("receive request from: {:?}", req.path);
    HttpResponse::new(200, None, Some(b"Hello, HTTP".to_vec()))
}
