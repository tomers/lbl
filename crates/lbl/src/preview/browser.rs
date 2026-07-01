//! Serve the preview bundle over HTTP and open it in a browser.
//!
//! ES-module based UIs (Nuxt/Vite) cannot load from `file://` in Chromium, so
//! `--open-browser` starts a tiny static server on loopback instead.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// Start a loopback static file server for `root`, open it in the browser, and
/// block until Ctrl+C.
pub fn serve_and_open(root: &Path) -> Result<()> {
    let server = PreviewServer::start(root)?;
    let url = server.url.clone();
    open_http_url(&url)?;
    eprintln!("Preview server: {url}");
    eprintln!("Press Ctrl+C to stop the preview server.");
    server.wait_until_ctrlc()
}

/// Hint printed when the bundle is written without `--open-browser`.
pub fn print_open_hint(bundle_dir: &Path) {
    eprintln!("Open the preview over HTTP (browsers block ES modules on file://), e.g.:");
    eprintln!(
        "  cd {} && python3 -m http.server 8080 --bind 127.0.0.1",
        bundle_dir.display()
    );
    eprintln!("Or re-run with --open-browser to serve and open automatically.");
}

struct PreviewServer {
    url: String,
    stop: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl PreviewServer {
    fn start(root: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("resolve preview directory {}", root.display()))?;
        let listener = TcpListener::bind("127.0.0.1:0").context("bind preview server")?;
        let port = listener
            .local_addr()
            .context("preview server address")?
            .port();
        let url = format!("http://127.0.0.1:{port}/");
        let (stop_tx, stop_rx) = mpsc::channel();

        let thread = thread::Builder::new()
            .name("lbl-preview-http".into())
            .spawn(move || run_server(listener, root, stop_rx))
            .context("spawn preview server thread")?;

        Ok(Self {
            url,
            stop: stop_tx,
            thread: Some(thread),
        })
    }

    fn wait_until_ctrlc(self) -> Result<()> {
        let (done_tx, done_rx) = mpsc::channel();
        ctrlc::set_handler(move || {
            let _ = done_tx.send(());
        })
        .context("install Ctrl+C handler")?;
        let _ = done_rx.recv();
        Ok(())
    }
}

impl Drop for PreviewServer {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(handle) = self.thread.take() {
            handle.join().ok();
        }
    }
}

fn run_server(listener: TcpListener, root: PathBuf, stop: mpsc::Receiver<()>) {
    listener.set_nonblocking(true).ok();
    loop {
        if stop.try_recv().is_ok() {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let root = root.clone();
                thread::spawn(move || {
                    let _ = handle_connection(stream, &root);
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(16));
            }
            Err(_) => break,
        }
    }
}

fn handle_connection(mut stream: TcpStream, root: &Path) -> Result<()> {
    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf).context("read preview request")?;
    if n == 0 {
        return Ok(());
    }
    let request = std::str::from_utf8(&buf[..n]).unwrap_or("");
    let Some(path) = parse_get_path(request) else {
        write_response(&mut stream, 400, "text/plain", b"Bad Request")?;
        return Ok(());
    };

    let file_path = match resolve_file(root, path) {
        Some(path) => path,
        None => {
            write_response(&mut stream, 404, "text/plain", b"Not Found")?;
            return Ok(());
        }
    };

    let body =
        std::fs::read(&file_path).with_context(|| format!("read {}", file_path.display()))?;
    let mime = mime_for_path(&file_path);
    write_response(&mut stream, 200, mime, &body)
}

fn parse_get_path(request: &str) -> Option<&str> {
    let line = request.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    if method != "GET" {
        return None;
    }
    let path = parts.next()?;
    Some(path.split('?').next().unwrap_or(path))
}

fn resolve_file(root: &Path, url_path: &str) -> Option<PathBuf> {
    let rel = url_path.trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.contains('\\') || rel.split('/').any(|part| part == "..") {
        return None;
    }
    let candidate = root.join(rel);
    let canonical = candidate.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    canonical.starts_with(&root).then_some(canonical)
}

fn mime_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {status_text}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Cache-Control: no-cache\r\n\
         \r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .context("write headers")?;
    stream.write_all(body).context("write body")?;
    Ok(())
}

fn open_http_url(url: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    let status = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("spawn xdg-open")?;

    #[cfg(target_os = "macos")]
    let status = Command::new("open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("spawn open")?;

    #[cfg(target_os = "windows")]
    let status = Command::new("cmd")
        .args(["/C", "start", "", url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("spawn start")?;

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = url;
        bail!("--open-browser is not supported on this platform");
    }

    if !status.success() {
        bail!("failed to open browser for {url}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_get_path() {
        assert_eq!(
            parse_get_path("GET /index.html HTTP/1.1"),
            Some("/index.html")
        );
        assert_eq!(
            parse_get_path("GET /_nuxt/app.js?x=1 HTTP/1.1"),
            Some("/_nuxt/app.js")
        );
    }

    #[test]
    fn resolves_paths_within_root() {
        let dir = std::env::temp_dir().join(format!("lbl-preview-serve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<html></html>").unwrap();
        let file = resolve_file(&dir, "/index.html").unwrap();
        assert_eq!(file, dir.join("index.html").canonicalize().unwrap());
        assert!(resolve_file(&dir, "/../etc/passwd").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn serves_index_over_http() {
        let dir = std::env::temp_dir().join(format!("lbl-preview-http-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<html>ok</html>").unwrap();

        let server = PreviewServer::start(&dir).unwrap();
        let url = server.url.clone();
        let body = fetch_url(&format!("{url}index.html"));
        assert!(body.contains("ok"), "body was: {body:?}");
        drop(server);

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn fetch_url(url: &str) -> String {
        use std::io::Read;
        use std::net::TcpStream;
        let url = url.strip_prefix("http://").unwrap();
        let (host_port, path) = url.split_once('/').unwrap_or((url, ""));
        let path = if path.is_empty() {
            "/"
        } else {
            &format!("/{path}")
        };
        let mut stream = TcpStream::connect(host_port).unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .unwrap();
        let mut buf = String::new();
        stream.read_to_string(&mut buf).unwrap();
        buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string()
    }
}
