use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = "127.0.0.1:8088";

    let mut stream = TcpStream::connect(addr).await?;
    let request = b"POST /first HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\r\nHELLOGET /second HTTP/1.1\r\nHost: localhost\r\n\r\nGET /third HTTP/1.1\r\nUser-Agent: terminal\r\n\r\n";

    stream.write_all(request).await?;

    let mut buf = [0; 4096];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => {
                println!("Connection closed");
                break;
            }
            Ok(n) => {
                println!("Received: {}", String::from_utf8_lossy(&buf[..n]));
            }
            Err(e) => {
                eprintln!("Failed to read from stream: {:?}", e);
                break;
            }
        }
    }

    Ok(())
}
