use clap::{arg, Parser};
use native_tls::Identity;
use std::fs;
use std::path::PathBuf;
use std::result::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_native_tls::TlsStream;
use url::Url;

enum Status {
    Success,
    TemporaryFailure,
    // PermanentFailure,
    ProxyRequestRefused,
    NotFound,
    BadRequest,
}

impl Status {
    fn code(&self) -> u8 {
        match self {
            Status::Success => 20,
            Status::TemporaryFailure => 40,
            // Status::PermanentFailure => 50,
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

fn process_request(request: String, config: &Config) -> Response {
    // NOTE: Default index.gmi in repo? "# Shooting Star\nThe shooting star server is up and running but there is nothing hosted here".to_string();

    match parse_request(request) {
        Ok(url) => {
            if url.scheme() != "gemini"
                || url.cannot_be_a_base()
                || url.port().is_some_and(|p| p != config.port)
            {
                return Response {
                    header: ResponseHeader::new(
                        Status::ProxyRequestRefused,
                        "Not a gemini request.",
                    ),
                    body: None,
                };
            }

            if url
                .host_str()
                .is_some_and(|h| !config.allowed_hosts.contains(&h.to_string()))
            {
                return Response {
                    header: ResponseHeader::new(
                        Status::ProxyRequestRefused,
                        "This host is not served here.",
                    ),
                    body: None,
                };
            }

            let mut read_path = PathBuf::from(&config.root);
            let mut path = url.path();

            if path == "/" || path.is_empty() {
                path = "/index.gmi";
            }

            read_path.push(path.trim_start_matches('/'));

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

async fn process_tls_stream(stream: &mut TlsStream<TcpStream>, config: &Config) {
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
                return;
            }
        };

        let response = process_request(request_line.to_string(), config);

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

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Host name. The default is 0.0.0.0.
    #[arg(short = 'H', long)]
    host: Option<String>,

    /// Port. The default is 1965.
    #[arg(short, long)]
    port: Option<u16>,

    /// Certificate
    #[arg(short, long)]
    cert: PathBuf,

    /// Private Key
    #[arg(short, long)]
    key: PathBuf,

    /// Document root
    #[arg(short, long)]
    root: PathBuf,

    /// List of additional allowed hostnames
    #[arg(short, long)]
    allowed_hosts: Vec<String>,
}

#[derive(Debug, Clone)]
struct Config {
    host: String,
    port: u16,
    cert: PathBuf,
    key: PathBuf,
    root: PathBuf,
    allowed_hosts: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let host = args.host.unwrap_or("0.0.0.0".to_string());
    let config = Config {
        host: host.clone(),
        port: args.port.unwrap_or(1965),
        cert: args.cert,
        key: args.key,
        root: args.root,
        allowed_hosts: {
            let mut allowed_hosts = args.allowed_hosts;
            allowed_hosts.push(host);
            allowed_hosts
        },
    };

    let addr = format!("{}:{}", config.host, config.port);
    let tcp: TcpListener = TcpListener::bind(&addr).await?;

    let cert_file = fs::read(&config.cert).expect("Error reading Certificate.");
    let key_file = fs::read(&config.key).expect("Error reading Key");
    let cert = Identity::from_pkcs8(&cert_file, &key_file)?;
    let tls_acceptor =
        tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::builder(cert).build()?);

    loop {
        let (socket, remote_addr) = tcp.accept().await.expect("error accepting tcp connection");
        let tls_acceptor = tls_acceptor.clone();
        let config = config.clone();
        println!("accept connection from {}", remote_addr);
        tokio::spawn(async move {
            // Accept the TLS connection.
            match tls_acceptor.accept(socket).await {
                Ok(mut stream) => {
                    process_tls_stream(&mut stream, &config).await;
                    stream.shutdown().await.expect("failed to shut down stream");
                }

                Err(e) => eprintln!("Connection from {remote_addr} closed: {e}"),
            }
        });
    }
}
