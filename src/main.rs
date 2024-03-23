use std::{
    io::{prelude::*, BufReader},
    net::{TcpListener, TcpStream},
};

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

fn main() -> std::io::Result<()> {
    let host = "127.0.0.1";
    let port = "1965";
    let listener = TcpListener::bind(format!("{host}:{port}"))?;

    println!("Server listening on {host}:{port}");

    // accept incoming connections and process them serially
    for stream in listener.incoming() {
        handle_connection(stream.unwrap());
    }

    Ok(())
}

fn handle_connection(mut stream: TcpStream) {
    let buf_reader = BufReader::new(&mut stream);
    if let Some(Ok(request_line)) = buf_reader.lines().next() {
        println!("Request: {:#?}", request_line);
        let response_header = check_request(&request_line);

        let mut response = response_header.render();
        let default_body = "#Welcome to gemini\nA server by Sal.";

        if let Status::Success = response_header.status {
            response.push_str(default_body);
        }

        stream.write_all(response.as_bytes()).unwrap();
    };
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

    ResponseHeader::new(Status::Success, "Ok")
}

// TODO:
// Response body function
// close connection??
// TLS moeilijkste denk ik.
// Of misschien multithreading? Tokio?
