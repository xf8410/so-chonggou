//! 诊断 HTTP 端点 —— Agora / 外部工具的取数口。
//!
//! **端口协商（重要）**：hlpatch SO 已占用 127.0.0.1:18765。本服务启动时按
//! 首选端口尝试 bind，被占自动顺延（18765→18766→…），并把实际端口写入
//! 外部媒体目录 + 宿主数据目录（都 best-effort），同时打日志 —— 两个 SO
//! 可以共存，Agora 指哪个端口就读哪个 SO 的数据。

use once_cell::sync::Lazy;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

static PORT: AtomicUsize = AtomicUsize::new(0);
static START: Lazy<Instant> = Lazy::new(Instant::now);

pub fn start(base_dir: Option<String>) {
    let spawned = std::thread::Builder::new()
        .name("chonggou-http".to_owned())
        .spawn(move || run(base_dir));
    if spawned.is_err() {
        crate::chlog!(error, "HTTP 诊断线程启动失败");
    }
}

pub fn port() -> u16 {
    PORT.load(Ordering::Relaxed) as u16
}

fn run(base_dir: Option<String>) {
    if !crate::config::get().http_enabled {
        crate::chlog!(info, "HTTP 诊断已在配置中关闭");
        return;
    }
    let preferred = crate::config::get().http_port;
    let candidates = [preferred, 18765, 18766, 18767, 18768];

    for port in candidates {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                PORT.store(port as usize, Ordering::Relaxed);
                crate::chlog!(info, "诊断 HTTP 已监听 127.0.0.1:{port} (端点: /health /status /logs /hooks /anim /config)");
                write_port_file(&base_dir, port);
                serve(listener);
                return;
            }
            Err(e) => {
                crate::chlog!(warn, "端口 {port} 不可用({e})，尝试下一个");
            }
        }
    }
    crate::chlog!(error, "诊断 HTTP 所有候选端口均被占用，放弃监听");
}

fn write_port_file(base_dir: &Option<String>, port: u16) {
    let _ = base_dir; // 路径统一从 config 取（含外部媒体目录）
    let paths = crate::config::port_file_paths();
    if paths.is_empty() {
        crate::chlog!(warn, "无数据目录，端口仅日志可见: {port}");
        return;
    }
    for p in paths {
        match std::fs::write(&p, format!("{port}\n")) {
            Ok(_) => crate::chlog!(info, "端口已写入 {}", p.display()),
            Err(e) => crate::chlog!(warn, "端口文件写入失败({e}): {}", p.display()),
        }
    }
}

fn serve(listener: TcpListener) {
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let _ = std::thread::Builder::new()
                    .name("chonggou-http-conn".to_owned())
                    .spawn(move || {
                        let _ = handle_conn(s);
                    });
            }
            Err(e) => crate::chlog!(warn, "accept 失败: {e}"),
        }
    }
}

fn handle_conn(stream: std::net::TcpStream) -> Option<()> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let path = line.split_whitespace().nth(1)?.to_string();

    // 排干请求头（Connection: close 前必须读完，否则对端半开）
    loop {
        let mut h = String::new();
        let n = reader.read_line(&mut h).ok()?;
        if n == 0 || h == "\r\n" || h == "\n" {
            break;
        }
    }

    let (route, query) = match path.split_once('?') {
        Some((r, q)) => (r.to_owned(), Some(q.to_owned())),
        None => (path.clone(), None),
    };

    let (code, body) = route_response(&route, query);
    respond(stream, code, body);
    Some(())
}

fn route_response(route: &str, query: Option<String>) -> (&'static str, String) {
    match route {
        "/health" => (
            "200 OK",
            serde_json::json!({
                "ok": true,
                "so": "so-chonggou",
                "version": env!("CARGO_PKG_VERSION"),
                "port": port(),
                "uptime_sec": START.elapsed().as_secs(),
            })
            .to_string(),
        ),
        "/status" => (
            "200 OK",
            serde_json::json!({
                "config": crate::config::get(),
                "log_entries": crate::logging::len(),
                "hooks_installed": crate::guard::own_hooks().len(),
            })
            .to_string(),
        ),
        "/logs" => {
            let n: usize = query
                .as_deref()
                .and_then(|q| q.split('&').find(|kv| kv.starts_with("n=")))
                .and_then(|kv| kv[2..].parse().ok())
                .unwrap_or(200);
            (
                "200 OK",
                serde_json::to_string(&crate::logging::recent(n)).unwrap_or_else(|_| "[]".to_owned()),
            )
        }
        "/hooks" => {
            let hooks: Vec<_> = crate::guard::own_hooks()
                .into_iter()
                .map(|(o, h)| serde_json::json!({"orig": format!("{o:#x}"), "hook": format!("{h:#x}")}))
                .collect();
            ("200 OK", serde_json::json!({ "count": hooks.len(), "hooks": hooks }).to_string())
        }
        "/anim" => ("200 OK", crate::training_anim::snapshot_json()),
        "/config" => (
            "200 OK",
            serde_json::json!({
                "config": crate::config::get(),
                "paths": {
                    "external": crate::config::external_dir(),
                    "host": crate::config::host_config_path().map(|p| p.display().to_string()),
                }
            })
            .to_string(),
        ),
        _ => ("404 Not Found", serde_json::json!({"error": "no such route", "routes": ["/health", "/status", "/logs?n=200", "/hooks", "/anim", "/config"]}).to_string()),
    }
}

fn respond(mut stream: std::net::TcpStream, code: &str, body: String) {
    let resp = format!(
        "HTTP/1.1 {code}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}
