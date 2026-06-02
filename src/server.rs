use std::collections::HashMap;
use std::fmt::Display;

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
}

#[derive(Debug)]
pub struct Server<'a> {
    pub addr: &'a str,
}

impl<'a> Server<'a> {
    pub fn new(addr: &'a str) -> Self {
        Self { addr }
    }

    pub async fn serve(&mut self) -> Result<(), ServerError> {
        println!("Server start serve on {}...", self.addr);

        let listener = TcpListener::bind(self.addr)
            .await
            .map_err(|e| ServerError::ListenError(e.to_string()))?;

        loop {
            let (mut stream, remote) = listener.accept().await?;
            println!("accept connection from: {}", remote);

            tokio::spawn(async move {
                match handle_stream(&mut stream).await {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("failed handle stream for: {}, error: {:?}", remote, e);
                    }
                }
            });
        }
    }
}

async fn handle_stream(stream: &mut TcpStream) -> Result<(), ServerError> {
    loop {
        let req = read_request(stream).await?;
        println!("received request: {}", req);
        let resp = HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: Some(b"OK\r\n".to_vec()),
        };
        write_response(stream, resp).await?;
    }
}

async fn read_request<R>(stream: &mut R) -> Result<HttpRequest, ServerError>
where
    R: AsyncRead + Unpin,
{
    let mut buf = BytesMut::with_capacity(4096);

    loop {
        let n_read = stream.read_buf(&mut buf).await?;
        if n_read == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
        }

        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut req = httparse::Request::new(&mut headers);
        match req.parse(&buf)? {
            Complete(n) => {
                let mut my_req = HttpRequest::from_http_parse(&req);
                my_req.set_body(&buf[n..]);
                return Ok(my_req);
            }
            Partial => continue,
        }
    }
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
    use crate::server::read_request;

    #[test]
    fn test_read_request() {
        let req = b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let mut stream = tokio_test::io::Builder::new().read(req).build();
        let my_req = tokio_test::block_on(read_request(&mut stream)).unwrap();

        assert_eq!(my_req.method, Some("GET".to_string()));
        assert_eq!(my_req.path, Some("/".to_string()));
        assert_eq!(my_req.headers.get("Host").unwrap(), &b"localhost".to_vec());
        assert_eq!(my_req.body, Some(vec![]));
    }
}
