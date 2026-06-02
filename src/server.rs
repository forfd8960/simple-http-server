use std::collections::HashMap;
use std::fmt::Display;
use std::io::ErrorKind::UnexpectedEof;

use bytes::BytesMut;
use httparse::Request;
use httparse::Status::{Complete, Partial};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::errors::ServerError;

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Option<String>,
    pub path: Option<String>,
    pub headers: HashMap<String, Vec<u8>>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, Vec<u8>>,
    pub body: Option<Vec<u8>>,
}

impl HttpResponse {
    pub fn new(status: u16, headers: HashMap<String, Vec<u8>>, body: Option<Vec<u8>>) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }

    pub fn new_entity_too_large() -> Self {
        Self {
            status: 413,
            headers: HashMap::new(),
            body: Some(b"Request Entity Too Large\r\n".to_vec()),
        }
    }
}

impl Display for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HttpRequest {{ method: {:?}, path: {:?}, headers: {:?}, body: {:?} }}",
            self.method, self.path, self.headers, self.body
        )
    }
}

impl HttpRequest {
    pub fn new() -> Self {
        Self {
            method: None,
            path: None,
            headers: HashMap::new(),
            body: None,
        }
    }

    pub fn is_body_over_limit(&self, limit: usize) -> bool {
        if let Some(body) = &self.body {
            body.len() > limit
        } else {
            false
        }
    }

    pub fn body_len(&self) -> usize {
        if let Some(body) = &self.body {
            body.len()
        } else {
            0
        }
    }

    pub fn from_http_parse(req: &Request<'_, '_>) -> Self {
        let mut headers = HashMap::new();

        for hdr in req.headers.iter().clone() {
            headers.insert(hdr.name.to_string(), hdr.value.to_vec());
        }

        Self {
            method: req.method.map(str::to_string),
            path: req.path.map(str::to_string),
            headers: headers,
            body: None,
        }
    }

    pub fn set_body(&mut self, body: &[u8]) {
        self.body = Some(body.to_vec())
    }

    pub fn append_body(&mut self, body: &[u8]) {
        if let Some(existing_body) = &mut self.body {
            existing_body.extend_from_slice(body);
        } else {
            self.body = Some(body.to_vec());
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub max_header_size: usize,
    pub max_body_size: usize,
}

#[derive(Debug, Clone)]
pub struct Server<'a> {
    pub addr: &'a str,
    pub config: ServerConfig,
}

impl<'a> Server<'a> {
    pub fn new(addr: &'a str, config: ServerConfig) -> Self {
        Self { addr, config }
    }

    pub async fn serve(&mut self) -> Result<(), ServerError> {
        println!("Server start serve on {}...", self.addr);

        let listener = TcpListener::bind(self.addr)
            .await
            .map_err(|e| ServerError::ListenError(e.to_string()))?;

        loop {
            let (mut stream, remote) = listener.accept().await?;
            println!("accept connection from: {}", remote);

            let config = self.config.clone();
            tokio::spawn(async move {
                match handle_stream(&mut stream, &config).await {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("failed handle stream for: {}, error: {:?}", remote, e);
                    }
                }
            });
        }
    }
}

async fn handle_stream(stream: &mut TcpStream, config: &ServerConfig) -> Result<(), ServerError> {
    loop {
        let mut req = read_request(stream, config).await?;
        println!("received request: {}", req);

        if req.is_body_over_limit(config.max_body_size) {
            write_response(stream, HttpResponse::new_entity_too_large()).await?;
            continue;
        }

        if let Some(content_length) = req.headers.get("Content-Length") {
            let content_length = String::from_utf8_lossy(content_length)
                .parse::<usize>()
                .map_err(|e| {
                    ServerError::ParseHeaderValueFailed(format!(
                        "failed parse Content-Length header value: {:?}, error: {}",
                        content_length, e
                    ))
                })?;

            if content_length > config.max_body_size {
                write_response(stream, HttpResponse::new_entity_too_large()).await?;
                continue;
            }

            let body = read_body(stream, content_length - req.body_len(), config).await?;
            req.append_body(&body);

            println!("received body: {:?}", req.body);
        }

        let resp = HttpResponse::new(200, HashMap::new(), Some(b"OK\r\n".to_vec()));
        write_response(stream, resp).await?;
    }
}

async fn read_request<R>(stream: &mut R, config: &ServerConfig) -> Result<HttpRequest, ServerError>
where
    R: AsyncRead + Unpin,
{
    let mut buf = BytesMut::with_capacity(4096);

    loop {
        let n_read = stream.read_buf(&mut buf).await?;
        if n_read == 0 {
            return Err(ServerError::IOError(UnexpectedEof.into()));
        }

        if buf.len() > config.max_header_size {
            return Err(ServerError::ReqSizeExceedLimit(
                "request size exceeds limit".to_string(),
            ));
        }

        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(&buf)? {
            Complete(n) => {
                let mut request = HttpRequest::from_http_parse(&req);
                request.set_body(&buf[n..]);
                return Ok(request);
            }
            Partial => continue,
        }
    }
}

async fn read_body<R>(
    stream: &mut R,
    content_length: usize,
    config: &ServerConfig,
) -> Result<Vec<u8>, ServerError>
where
    R: AsyncRead + Unpin,
{
    if content_length > config.max_body_size {
        return Err(ServerError::ReqSizeExceedLimit(
            "request body size exceeds limit".to_string(),
        ));
    }

    let mut body = vec![0; content_length];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

async fn write_response<W>(stream: &mut W, resp: HttpResponse) -> Result<(), ServerError>
where
    W: AsyncWriteExt + Unpin,
{
    let mut response = format!("HTTP/1.1 {} OK\r\n", resp.status).into_bytes();

    for (k, v) in resp.headers.iter() {
        response.extend_from_slice(k.as_bytes());
        response.extend_from_slice(b": ");
        response.extend_from_slice(v);
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(b"Content-Length: ");
    if let Some(body) = &resp.body {
        response.extend_from_slice(body.len().to_string().as_bytes());
    } else {
        response.extend_from_slice(b"0");
    }

    response.extend_from_slice(b"\r\n\r\n");
    if let Some(body) = resp.body {
        response.extend_from_slice(&body);
    }
    stream.write_all(&response).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::server::{ServerConfig, read_request};

    #[test]
    fn test_read_request() {
        let req = b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let mut stream = tokio_test::io::Builder::new().read(req).build();
        let my_req = tokio_test::block_on(read_request(
            &mut stream,
            &ServerConfig {
                max_header_size: 8192,
                max_body_size: 8192,
            },
        ))
        .unwrap();

        assert_eq!(my_req.method, Some("GET".to_string()));
        assert_eq!(my_req.path, Some("/".to_string()));
        assert_eq!(my_req.headers.get("Host").unwrap(), &b"localhost".to_vec());
        assert_eq!(my_req.body, Some(vec![]));
    }
}
