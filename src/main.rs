use native_tls::Identity;
use std::fs;
use std::path::PathBuf;
use std::result::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_native_tls::TlsStream;
use url::Url;

enum Status {
    Success,
    TemporaryFailure,
    PermanentFailure,
    ProxyRequestRefused,
    NotFound,
    BadRequest,
}

impl Status {
    fn code(&self) -> u8 {
        match self {
            Status::Success => 20,
            Status::TemporaryFailure => 40,
            Status::PermanentFailure => 50,
            Status::ProxyRequestRefused => 53,
            Status::NotFound => 51,
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

struct Response {
    header: ResponseHeader,
    body: Option<String>,
}
impl Response {
    fn render(&self) -> String {
        match self.header.status {
            Status::Success => {
                self.header.render()
                    + match &self.body {
                        Some(s) => s,
                        None => "",
                    }
            }
            _ => self.header.render(),
        }
    }
}

fn parse_request(request_line: String) -> Result<url::Url, &'static str> {
    if request_line.starts_with('﻿') {
        Err("The request MUST NOT begin with a U+FEFF byte order mark.")
    } else if request_line.len() > 1024 {
        Err("URL is too long. Maximum length is 1024 bytes.")
    } else {
        match Url::parse(&request_line) {
            Ok(u) => Ok(u),
            Err(_) => Err("Error parsing the url"),
        }
    }
}

fn process_request(request: String) -> Response {
    // TODO: Default index.gmi in repo? "# Shooting Star\nThe shooting star server is up and running but there is nothing hosted here".to_string();

    // TODO: allowed hosts config
    match parse_request(request) {
        Ok(url) => {
            if url.scheme() != "gemini"
                || url.cannot_be_a_base()
                || url.port().is_some_and(|p| p != 1965)
            {
                return Response {
                    header: ResponseHeader::new(
                        Status::ProxyRequestRefused,
                        "Not a gemini request.",
                    ),
                    body: None,
                };
            }

            let root_dir = "./documents";

            let mut read_path = PathBuf::from(root_dir);

            let mut path = url.path();

            println!("{path}");
            if path == "/" || path.is_empty() {
                path = "/index.gmi";
            }

            read_path.push(path.trim_start_matches('/'));

            println!(
                "{path} Read Path: {:?} {}",
                read_path,
                path.trim_start_matches('/')
            );

            if !read_path.exists() {
                return Response {
                    header: ResponseHeader::new(Status::NotFound, "Not Found"),
                    body: None,
                };
            }

            if let Ok(body) = fs::read_to_string(read_path) {
                Response {
                    header: ResponseHeader::new(Status::Success, "text/gemini"),
                    body: Some(body),
                }
            } else {
                Response {
                    header: ResponseHeader::new(Status::TemporaryFailure, "Internal Server Error"),
                    body: None,
                }
            }
        }
        Err(err) => Response {
            header: ResponseHeader::new(Status::BadRequest, err),
            body: None,
        },
    }
}

async fn process_tls_stream(stream: &mut TlsStream<TcpStream>) {
    let mut buffer = [0; 1026]; // 1024 for the url + CRLF
    let n = stream
        .read(&mut buffer)
        .await
        .expect("Error reading first line of stream.");

    if n == 0 {
        return;
    }

    if let Ok(raw_line) = String::from_utf8(buffer[0..n].into()) {
        let request_line = match raw_line.split_once("\r\n") {
            Some((l, _)) => l,
            None => {
                println!("ja");
                return;
            }
        };

        let response = process_request(request_line.to_string());

        match response.header.status {
            Status::Success => {
                println!("Request: [{}] {}", Status::Success.code(), &request_line);
            }
            Status::BadRequest => println!("BadRequest: {request_line}"),
            Status::NotFound => println!("Not found: {request_line}"),
            _ => println!("Not able to process request: {request_line}"),
        }

        stream
            .write_all(response.render().as_bytes())
            .await
            .unwrap();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:1965".to_string();
    let tcp: TcpListener = TcpListener::bind(&addr).await?;

    let cert = Identity::from_pkcs8(
        // TODO: Certificate is included at compile time for now but I want tot use arguments in the future.
        include_bytes!("../keys/gemini.svw.li.crt"),
        include_bytes!("../keys/gemini.svw.li.key"),
    )?;
    let tls_acceptor =
        tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::builder(cert).build()?);

    loop {
        let (socket, remote_addr) = tcp.accept().await.expect("error accepting tcp connection");
        let tls_acceptor = tls_acceptor.clone();
        println!("accept connection from {}", remote_addr);
        tokio::spawn(async move {
            // Accept the TLS connection.
            match tls_acceptor.accept(socket).await {
                Ok(mut stream) => {
                    process_tls_stream(&mut stream).await;
                    stream.shutdown().await.expect("failed to shut down stream");
                }

                Err(e) => eprintln!("Connection from {remote_addr} closed: {e}"),
            }
        });
    }
}
