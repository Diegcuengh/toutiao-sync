use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use tiny_http::{Header, Method, Response, Server, StatusCode};

use crate::error::AppError;

const DEBUG_HOST: &str = "127.0.0.1";
const DEBUG_PORT: u16 = 14321;

pub fn ensure_debug_frontend_server() -> Result<(), AppError> {
    if !cfg!(debug_assertions) {
        return Ok(());
    }

    if is_debug_server_ready() {
        return Ok(());
    }

    let root_dir = resolve_debug_dist_dir().ok_or_else(|| {
        AppError::Message("未找到 debug 前端静态资源目录，无法启动内置页面服务。".into())
    })?;

    let server = Server::http(format!("{DEBUG_HOST}:{DEBUG_PORT}"))
        .map_err(|error| AppError::Message(format!("启动内置页面服务失败: {error}")))?;

    thread::spawn(move || run_server(server, root_dir));

    for _ in 0..40 {
        if is_debug_server_ready() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    Err(AppError::Message(
        "内置页面服务启动超时，127.0.0.1:14321 仍不可用。".into(),
    ))
}

fn run_server(server: Server, root_dir: PathBuf) {
    for request in server.incoming_requests() {
        if request.method() != &Method::Get && request.method() != &Method::Head {
            let response = Response::empty(StatusCode(405));
            let _ = request.respond(response);
            continue;
        }

        let url_path = request.url().split('?').next().unwrap_or("/");
        let candidate = map_request_path(&root_dir, url_path);
        let file_path = if candidate.exists() {
            candidate
        } else {
            root_dir.join("index.html")
        };

        match fs::File::open(&file_path) {
            Ok(mut file) => {
                let mut bytes = Vec::new();
                if file.read_to_end(&mut bytes).is_err() {
                    let _ = request.respond(Response::empty(StatusCode(500)));
                    continue;
                }
                let mut response = Response::from_data(bytes);
                if let Ok(header) =
                    Header::from_bytes(&b"Content-Type"[..], content_type_for(&file_path).as_bytes())
                {
                    response = response.with_header(header);
                }
                let _ = request.respond(response);
            }
            Err(_) => {
                let _ = request.respond(Response::empty(StatusCode(404)));
            }
        }
    }
}

fn map_request_path(root_dir: &Path, request_path: &str) -> PathBuf {
    let trimmed = request_path.trim_start_matches('/');
    if trimmed.is_empty() {
        return root_dir.join("index.html");
    }
    root_dir.join(trimmed.replace('/', "\\"))
}

fn content_type_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
    {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn is_debug_server_ready() -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], DEBUG_PORT));
    let mut stream = match TcpStream::connect_timeout(&address, Duration::from_millis(200)) {
        Ok(stream) => stream,
        Err(_) => return false,
    };

    let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(300)));

    if stream
        .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }

    let mut buffer = [0u8; 128];
    match stream.read(&mut buffer) {
        Ok(read_size) if read_size > 0 => std::str::from_utf8(&buffer[..read_size])
            .map(|response| response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200"))
            .unwrap_or(false),
        _ => false,
    }
}

fn resolve_debug_dist_dir() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();

    let candidates = [
        current_dir.join("embedded-dist"),
        exe_dir.join("embedded-dist"),
        current_dir.join("dist"),
        exe_dir
            .parent()
            .map(|path| path.join("dist"))
            .unwrap_or_else(|| exe_dir.join("dist")),
    ];

    candidates
        .into_iter()
        .find(|path| path.join("index.html").exists())
}
