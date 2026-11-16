//! Minimal read-only HTTP server for a generated review site. It binds to
//! loopback by default, serves only files inside the selected directory and
//! never invokes a network or a reviewed program.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

const MAX_REQUEST_LINE: usize = 8192;
const READ_TIMEOUT: Duration = Duration::from_secs(10);

pub fn serve(directory: &Path, host: &str, port: u16, open: bool) -> Result<(), String> {
    if !directory.is_dir() {
        return Err(format!("{} is not a directory", directory.display()));
    }
    if !directory.join("index.html").is_file() && !directory.join("index.json").is_file() {
        return Err(format!(
            "{} does not contain index.html or index.json; run `csb review` first",
            directory.display()
        ));
    }
    let root = directory
        .canonicalize()
        .map_err(|error| format!("Cannot read {}: {error}", directory.display()))?;
    let listener = TcpListener::bind((host, port))
        .map_err(|error| format!("Cannot bind {host}:{port}: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("Cannot read listener address: {error}"))?;
    let url = format!("http://{address}/");
    println!("{url}");
    let _ = std::io::stdout().flush();
    if open {
        open_browser(&url);
    }
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let root = root.clone();
                std::thread::spawn(move || handle(stream, &root));
            }
            Err(error) => eprintln!("csb: connection failed: {error}"),
        }
    }
    Ok(())
}

fn handle(mut stream: TcpStream, root: &Path) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(READ_TIMEOUT));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    });
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.len() > MAX_REQUEST_LINE {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if !matches!(method, "GET" | "HEAD") {
        respond(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            b"Method not allowed\n",
            method == "HEAD",
        );
        return;
    }
    // Drain headers so the response does not race a client write.
    let mut header = String::new();
    loop {
        header.clear();
        match reader.read_line(&mut header) {
            Ok(0) | Err(_) => break,
            Ok(_) if header == "\r\n" || header == "\n" => break,
            Ok(_) => {}
        }
    }
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let relative = match decode(path) {
        Some(value) => value,
        None => {
            respond(
                &mut stream,
                400,
                "text/plain; charset=utf-8",
                b"Bad request\n",
                method == "HEAD",
            );
            return;
        }
    };
    let relative = relative.trim_start_matches('/');
    let relative = if relative.is_empty() {
        "index.html"
    } else {
        relative
    };
    if relative
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == ".." || part == ".git")
    {
        respond(
            &mut stream,
            404,
            "text/plain; charset=utf-8",
            b"Not found\n",
            method == "HEAD",
        );
        return;
    }
    let candidate = root.join(relative);
    let resolved = match candidate.canonicalize() {
        Ok(path) if path.starts_with(root) && path.is_file() => path,
        _ => {
            respond(
                &mut stream,
                404,
                "text/plain; charset=utf-8",
                b"Not found\n",
                method == "HEAD",
            );
            return;
        }
    };
    let bytes = match fs::read(&resolved) {
        Ok(bytes) => bytes,
        Err(_) => {
            respond(
                &mut stream,
                404,
                "text/plain; charset=utf-8",
                b"Not found\n",
                method == "HEAD",
            );
            return;
        }
    };
    respond(
        &mut stream,
        200,
        content_type(&resolved),
        &bytes,
        method == "HEAD",
    );
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8], head_only: bool) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    if !head_only {
        let _ = stream.write_all(body);
    }
    let _ = stream.flush();
}

fn decode(target: &str) -> Option<String> {
    let mut out = Vec::with_capacity(target.len());
    let bytes = target.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = *bytes.get(index + 1)?;
                let low = *bytes.get(index + 2)?;
                out.push((hex(high)? << 4) | hex(low)?);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out)
        .ok()
        .filter(|value| !value.contains('\0'))
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "json" => "application/json",
        "md" => "text/markdown; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let (program, arguments) = ("open", vec![url]);
    #[cfg(target_os = "windows")]
    let (program, arguments) = ("cmd", vec!["/C", "start", "", url]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let (program, arguments) = ("xdg-open", vec![url]);
    let _ = std::process::Command::new(program).args(arguments).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_paths_and_rejects_bad_escapes() {
        assert_eq!(decode("/a%20b.json").as_deref(), Some("/a b.json"));
        assert_eq!(decode("/%2e%2e/secret").as_deref(), Some("/../secret"));
        assert_eq!(decode("/bad%2"), None);
        assert_eq!(decode("/bad%zz"), None);
    }

    #[test]
    fn content_types_cover_review_artifacts() {
        assert_eq!(
            content_type(Path::new("index.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(content_type(Path::new("index.json")), "application/json");
        assert_eq!(
            content_type(Path::new("a.unknown")),
            "application/octet-stream"
        );
    }
}
