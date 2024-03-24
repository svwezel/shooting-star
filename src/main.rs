use native_tls::Identity;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use url::Url;

enum Status {
    Success,
    PermanentFailure,
    BadRequest,
}

impl Status {
    fn code(&self) -> u8 {
        match self {
            Status::Success => 20,
            Status::PermanentFailure => 50,
            Status::BadRequest => 59,
        }
    }
}

struct ResponseHeader {
    status: Status,
    meta: String,
}

impl ResponseHeader {
    fn new(status: Status, meta: &str) -> ResponseHeader {
        ResponseHeader {
            status,
            meta: String::from(meta),
        }
    }

    fn render(&self) -> String {
        format!("{} {}\r\n", self.status.code(), &self.meta)
    }
}

fn check_request(request_line: &String) -> ResponseHeader {
    if request_line.starts_with('﻿') {
        return ResponseHeader::new(
            Status::BadRequest,
            "The request MUST NOT begin with a U+FEFF byte order mark.",
        );
    }

    if request_line.len() > 1024 {
        return ResponseHeader::new(
            Status::BadRequest,
            "URL is too long. Maximum length is 1024 bytes.",
        );
    }

    let url = match Url::parse(request_line) {
        Ok(url) => url,
        Err(_) => {
            return ResponseHeader::new(Status::BadRequest, "Something went wrong parsing the URL.")
        }
    };

    if url.scheme() != "gemini" {
        return ResponseHeader::new(Status::PermanentFailure, "Not a gemini request.");
    }

    ResponseHeader::new(Status::Success, "text/gemini")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Bind the server's socket
    let addr = "0.0.0.0:1965".to_string();
    let tcp: TcpListener = TcpListener::bind(&addr).await?;

    let cert = Identity::from_pkcs8(
        include_bytes!("../keys/gemini.svw.li.crt"),
        include_bytes!("../keys/gemini.svw.li.key"),
    )?;
    let tls_acceptor =
        tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::builder(cert).build()?);

    loop {
        // Asynchronously wait for an inbound socket.
        let (socket, remote_addr) = tcp.accept().await?;
        let tls_acceptor = tls_acceptor.clone();
        println!("accept connection from {}", remote_addr);
        tokio::spawn(async move {
            // Accept the TLS connection.
            let mut tls_stream = tls_acceptor.accept(socket).await.expect("accept error");

            let buf_reader = BufReader::new(&mut tls_stream);
            let mut lines = buf_reader.lines();
            let first_line = lines
                .next_line()
                .await
                .expect("Error reading first line of stream.");

            if let Some(request_line) = first_line {
                let response_header = check_request(&request_line);
                let default_body = "#Sal's Gemini server\nWelcome to the Sal gemini server. There is still a lot to implement.\r\n";

                let mut response = response_header.render();

                match response_header.status {
                    Status::Success => {
                        println!("Request: [{}] {}", Status::Success.code(), &request_line);
                        response.push_str(default_body);
                    }
                    Status::BadRequest => println!("BadRequest: {}", &request_line),
                    _ => println!("Not able to process request: {}", &request_line),
                }

                tls_stream.write_all(response.as_bytes()).await.unwrap();
            }

            tls_stream
                .shutdown()
                .await
                .expect("failed to shut down stream");
        });
    }
}
