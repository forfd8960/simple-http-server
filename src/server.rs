use tokio::net::{TcpListener, TcpStream};

use crate::errors::ServerError;

#[derive(Debug)]
pub struct Server<'a> {
    pub addr: &'a str,
}

impl<'a> Server<'a> {
    pub fn new(addr: &'a str) -> Self {
        Self { addr }
    }

    pub async fn serve(&mut self) -> Result<(), ServerError> {
        println!("Server start serve...");

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
    Ok(())
}
