use std::{collections::HashMap, fmt::Display};

use httparse::Request;

#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: Option<String>,
    pub path: Option<String>,
    pub headers: HashMap<String, Vec<u8>>,
    pub params: HashMap<String, String>,
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
    pub fn new() -> Self {
        Self {
            method: None,
            path: None,
            headers: HashMap::new(),
            params: HashMap::new(),
            body: None,
        }
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.get(name).map(|s| s.as_str())
    }

    pub fn header(&self, name: &str) -> Option<Vec<u8>> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
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
            params: HashMap::new(),
            body: None,
        }
    }

    pub fn set_body(&mut self, body: &[u8]) {
        self.body = Some(body.to_vec())
    }
}

impl HttpResponse {
    pub fn new(
        status: u16,
        headers: Option<HashMap<String, Vec<u8>>>,
        body: Option<Vec<u8>>,
    ) -> Self {
        Self {
            status,
            headers: headers.unwrap_or_else(HashMap::new),
            body,
        }
    }

    pub fn new_not_found(message: &str) -> Self {
        Self {
            status: 404,
            headers: HashMap::new(),
            body: Some(format!("Not Found: {}\r\n", message).into_bytes()),
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
