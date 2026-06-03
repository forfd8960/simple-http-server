use std::{collections::HashMap, pin::Pin};

use regex::Regex;

use crate::req_res::{HttpRequest, HttpResponse};

// Handler is an async function that takes request and returns response
pub type Handler =
    Box<dyn Fn(HttpRequest) -> Pin<Box<dyn Future<Output = HttpResponse> + Send>> + Send + Sync>;

pub struct Route {
    pattern: Regex,
    params: Vec<String>,
    handler: Handler,
}

pub struct Router {
    // Vec of (method, route)
    routes: Vec<(String, Route)>,
}

impl Router {
    pub fn new() -> Self {
        Router { routes: Vec::new() }
    }

    pub fn get<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(HttpRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HttpResponse> + Send + 'static,
    {
        self.add_route("GET", path, handler);
    }

    pub fn put<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(HttpRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HttpResponse> + Send + 'static,
    {
        self.add_route("PUT", path, handler);
    }

    pub fn post<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(HttpRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HttpResponse> + Send + 'static,
    {
        self.add_route("POST", path, handler);
    }

    pub fn delete<F, Fut>(&mut self, path: &str, handler: F)
    where
        F: Fn(HttpRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HttpResponse> + Send + 'static,
    {
        self.add_route("DELETE", path, handler);
    }

    fn add_route<F, Fut>(&mut self, method: &str, path: &str, handler: F)
    where
        F: Fn(HttpRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HttpResponse> + Send + 'static,
    {
        println!("adding route: method: {}, path: {}", method, path);
        let (pattern, params) = path_to_regex(path);

        println!("route pattern: {}, params: {:?}", pattern, params);
        self.routes.push((
            method.to_string(),
            Route {
                pattern,
                params,
                handler: Box::new(move |req| Box::pin(handler(req))),
            },
        ));
    }

    pub async fn route(&self, mut req: HttpRequest) -> HttpResponse {
        println!(
            "calling route with req path: {:?}, method: {:?}",
            req.path, req.method
        );

        for (method, route) in &self.routes {
            let req_method = req.method.clone();
            if req_method.is_none() || req.path.is_none() {
                continue;
            }

            println!(
                "checking if req method is equal to route method: {}, {}",
                req_method.as_ref().unwrap(),
                method
            );

            if req_method.unwrap() != *method {
                continue;
            }

            let path = req.path.as_deref().unwrap_or("");

            println!(
                "checking if req path matches route pattern: {}, {}",
                path, route.pattern
            );

            if let Some(caps) = route.pattern.captures(path) {
                println!("pattern matched, extracting params...");

                let mut params = HashMap::new();

                for (i, name) in route.params.iter().enumerate() {
                    if let Some(value) = caps.get(i + 1) {
                        params.insert(name.clone(), value.as_str().to_string());
                    }
                }

                req.params = params;
                return (route.handler)(req).await;
            }
        }

        HttpResponse::new_not_found("Route not found")
    }
}

fn path_to_regex(path: &str) -> (Regex, Vec<String>) {
    let mut pattern = String::from("^");
    let mut params = Vec::new();

    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }

        pattern.push('/');

        if segment.starts_with(':') {
            pattern.push_str("([^/]+)");
            params.push(segment[1..].to_string());
        } else {
            pattern.push_str(&regex::escape(segment));
        }
    }
    pattern.push_str("$");
    (Regex::new(&pattern).unwrap(), params)
}
