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

    pub fn new_bad_request(message: &str) -> Self {
        Self {
            status: 400,
            headers: HashMap::new(),
            body: Some(format!("Bad Request: {}\r\n", message).into_bytes()),
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
            headers.insert(hdr.name.to_string().to_lowercase(), hdr.value.to_vec());
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
    let mut conn_buf = BytesMut::with_capacity(4096);

    loop {
        let (mut req, header_len) = read_request(stream, &mut conn_buf, config).await?;
        println!("received request: {}", req);

        handle_content(stream, &mut conn_buf, header_len, &mut req, config).await?;

        let resp = HttpResponse::new(200, HashMap::new(), Some(b"OK".to_vec()));
        write_response(stream, resp).await?;
    }
}

async fn handle_content(
    stream: &mut TcpStream,
    conn_buf: &mut BytesMut,
    header_len: usize,
    req: &mut HttpRequest,
    config: &ServerConfig,
) -> Result<(), ServerError> {
    let c_l = req.headers.get("content-length");
    if c_l.is_none() {
        req.set_body(&[]);
        let _ = conn_buf.split_to(header_len);
        return Ok(());
    }

    let len_bs = c_l.unwrap();
    let content_length = match String::from_utf8_lossy(len_bs).parse::<usize>() {
        Ok(len) => len,
        Err(_) => {
            write_response(
                stream,
                HttpResponse::new_bad_request("invalid Content-Length header value"),
            )
            .await?;
            return Ok(());
        }
    };

    if content_length > config.max_body_size {
        write_response(stream, HttpResponse::new_entity_too_large()).await?;
        discard_current_request_body(stream, conn_buf, header_len, content_length).await?;
        return Ok(());
    }

    let total_needed = header_len + content_length;
    ensure_buffer_len(stream, conn_buf, total_needed).await?;

    req.set_body(&conn_buf[header_len..total_needed]);
    let _ = conn_buf.split_to(total_needed);

    println!("received body: {:?}", req.body);

    Ok(())
}

async fn read_request<R>(
    stream: &mut R,
    conn_buf: &mut BytesMut,
    config: &ServerConfig,
) -> Result<(HttpRequest, usize), ServerError>
where
    R: AsyncRead + Unpin,
{
    loop {
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(conn_buf)? {
            Complete(n) => {
                let request = HttpRequest::from_http_parse(&req);
                return Ok((request, n));
            }
            Partial => {
                if conn_buf.len() > config.max_header_size {
                    return Err(ServerError::ReqSizeExceedLimit(
                        "request header size exceeds limit".to_string(),
                    ));
                }

                let n_read = stream.read_buf(conn_buf).await?;
                if n_read == 0 {
                    return Err(ServerError::IOError(UnexpectedEof.into()));
                }
                continue;
            }
        }
    }
}

async fn ensure_buffer_len<R>(
    stream: &mut R,
    conn_buf: &mut BytesMut,
    target_len: usize,
) -> Result<(), ServerError>
where
    R: AsyncRead + Unpin,
{
    while conn_buf.len() < target_len {
        let n_read = stream.read_buf(conn_buf).await?;
        if n_read == 0 {
            return Err(ServerError::IOError(UnexpectedEof.into()));
        }
    }
    Ok(())
}

async fn discard_current_request_body<R>(
    stream: &mut R,
    conn_buf: &mut BytesMut,
    header_len: usize,
    content_length: usize,
) -> Result<(), ServerError>
where
    R: AsyncRead + Unpin,
{
    let total_needed = header_len + content_length;
    if conn_buf.len() >= total_needed {
        let _ = conn_buf.split_to(total_needed);
        return Ok(());
    }

    let already_buffered_body = conn_buf.len().saturating_sub(header_len);
    let mut remaining = content_length.saturating_sub(already_buffered_body);
    conn_buf.clear();

    let mut scratch = [0u8; 4096];
    while remaining > 0 {
        let read_len = remaining.min(scratch.len());
        let n_read = stream.read(&mut scratch[..read_len]).await?;
        if n_read == 0 {
            return Err(ServerError::IOError(UnexpectedEof.into()));
        }
        remaining -= n_read;
    }

    Ok(())
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
    use bytes::BytesMut;

    use crate::server::{ServerConfig, ensure_buffer_len, read_request};

    #[test]
    fn test_read_request() {
        let req = b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let mut stream = tokio_test::io::Builder::new().read(req).build();
        let mut conn_buf = BytesMut::new();
        let (my_req, _header_len) = tokio_test::block_on(read_request(
            &mut stream,
            &mut conn_buf,
            &ServerConfig {
                max_header_size: 8192,
                max_body_size: 8192,
            },
        ))
        .unwrap();

        assert_eq!(my_req.method, Some("GET".to_string()));
        assert_eq!(my_req.path, Some("/".to_string()));
        assert_eq!(my_req.headers.get("host").unwrap(), &b"localhost".to_vec());
        assert_eq!(my_req.body, None);
    }

    #[test]
    fn test_read_request_pipelined_two_requests_in_one_packet() {
        let req = b"GET /first HTTP/1.1\r\nHost: localhost\r\n\r\nGET /second HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let mut stream = tokio_test::io::Builder::new().read(req).build();
        let mut conn_buf = BytesMut::new();
        let cfg = ServerConfig {
            max_header_size: 8192,
            max_body_size: 8192,
        };

        let (req1, header_len1) =
            tokio_test::block_on(read_request(&mut stream, &mut conn_buf, &cfg)).unwrap();

        assert_eq!(req1.method, Some("GET".to_string()));
        assert_eq!(req1.path, Some("/first".to_string()));
        let _ = conn_buf.split_to(header_len1);

        let (req2, _header_len2) =
            tokio_test::block_on(read_request(&mut stream, &mut conn_buf, &cfg)).unwrap();

        assert_eq!(req2.method, Some("GET".to_string()));
        assert_eq!(req2.path, Some("/second".to_string()));
    }

    #[test]
    fn test_read_request_pipelined_first_has_content_length_body() {
        let req = b"POST /first HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\r\nHELLOGET /second HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let mut stream = tokio_test::io::Builder::new().read(req).build();
        let mut conn_buf = BytesMut::new();
        let cfg = ServerConfig {
            max_header_size: 8192,
            max_body_size: 8192,
        };

        let (mut req1, header_len1) =
            tokio_test::block_on(read_request(&mut stream, &mut conn_buf, &cfg)).unwrap();

        let content_length1 = String::from_utf8_lossy(req1.headers.get("content-length").unwrap())
            .parse::<usize>()
            .unwrap();
        let total_needed1 = header_len1 + content_length1;
        tokio_test::block_on(ensure_buffer_len(&mut stream, &mut conn_buf, total_needed1)).unwrap();

        assert!(conn_buf.len() >= total_needed1);

        req1.set_body(&conn_buf[header_len1..total_needed1]);
        let _ = conn_buf.split_to(total_needed1);

        assert_eq!(req1.method, Some("POST".to_string()));
        assert_eq!(req1.path, Some("/first".to_string()));
        assert_eq!(req1.body, Some(b"HELLO".to_vec()));

        let (req2, header_len2) =
            tokio_test::block_on(read_request(&mut stream, &mut conn_buf, &cfg)).unwrap();
        assert_eq!(req2.method, Some("GET".to_string()));
        assert_eq!(req2.path, Some("/second".to_string()));

        let _ = conn_buf.split_to(header_len2);
    }
}
