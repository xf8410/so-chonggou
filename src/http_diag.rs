//! 诊断 HTTP 端点 —— Agora / 外部工具的取数口。
//!
//! **端口协商（重要）**：hlpatch SO 已占用 127.0.0.1:18765。本服务启动时按
//! 首选端口尝试 bind，被占自动顺延（18765→18766→…），并把实际端口写入
//! 外部媒体目录 + 宿主数据目录（都 best-effort），同时打日志 —— 两个 SO
//! 可以共存，Agora 指哪个端口就读哪个 SO 的数据。
//!
//! v0.3.0 路由表新增：
//! - `GET /uitree`        读 RectTransform 位置树（文字位置不对排查）
//! - `GET /uitree/refresh` 重新抓一次再返回
//! - `POST /config`       改配置（body 为 JSON 对象）；返回改后的完整配置

use once_cell::sync::Lazy;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

static PORT: AtomicUsize = AtomicUsize::new(0);
static START: Lazy<Instant> = Lazy::new(Instant::now);

const ROUTES: &str = "/health /status /logs /hooks /anim /uitree /uitree/refresh /config";

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
                crate::chlog!(info, "诊断 HTTP 已监听 127.0.0.1:{port} (端点: {ROUTES})");
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

struct Req {
    method: String,
    path: String,
    body: String,
}

fn read_request(reader: &mut BufReader<std::net::TcpStream>) -> Option<Req> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();

    let mut content_length = 0usize;
    loop {
        let mut h = String::new();
        let n = reader.read_line(&mut h).ok()?;
        if n == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }

    // 封顶 64 KiB —— 诊断服务不该被一个畸形 Content-Length 撑爆内存
    let mut body = vec![0u8; content_length.min(64 * 1024)];
    if !body.is_empty() {
        reader.read_exact(&mut body).ok()?;
    }
    Some(Req {
        method,
        path,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn handle_conn(stream: std::net::TcpStream) -> Option<()> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let req = read_request(&mut reader)?;
    let (code, body) = route_response(&req);
    respond(stream, code, body);
    Some(())
}

fn route_response(req: &Req) -> (&'static str, String) {
    let (route, query) = match req.path.split_once('?') {
        Some((r, q)) => (r, Some(q.to_owned())),
        None => (req.path.as_str(), None),
    };

    match (req.method.as_str(), route) {
        ("GET", "/health") => (
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
        ("GET", "/status") => (
            "200 OK",
            serde_json::json!({
                "config": crate::config::get(),
                "log_entries": crate::logging::len(),
                "hooks_installed": crate::guard::own_hooks().len(),
            })
            .to_string(),
        ),
        ("GET", "/logs") => {
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
        ("GET", "/hooks") => {
            let hooks: Vec<_> = crate::guard::own_hooks()
                .into_iter()
                .map(|(o, h)| serde_json::json!({"orig": format!("{o:#x}"), "hook": format!("{h:#x}")}))
                .collect();
            ("200 OK", serde_json::json!({ "count": hooks.len(), "hooks": hooks }).to_string())
        }
        ("GET", "/anim") => ("200 OK", crate::training_anim::snapshot_json()),
        ("GET", "/uitree") => ("200 OK", crate::ui_probe::snapshot_json()),
        ("GET", "/uitree/refresh") => {
            crate::ui_probe::refresh();
            ("200 OK", crate::ui_probe::snapshot_json())
        }
        ("POST", "/config") => match serde_json::from_str::<serde_json::Value>(&req.body) {
            Err(e) => (
                "400 Bad Request",
                serde_json::json!({"error": format!("body 不是合法 JSON: {e}")}).to_string(),
            ),
            Ok(v) => {
                if let Some(obj) = v.as_object() {
                    let mut bad = Vec::new();
                    crate::config::update(|c| {
                        for (k, val) in obj {
                            match k.as_str() {
                                "http_enabled" => {
                                    c.http_enabled = val.as_bool().unwrap_or(c.http_enabled)
                                }
                                "http_port" => {
                                    if let Some(p) = val.as_u64() {
                                        if (1024..=65535).contains(&p) {
                                            c.http_port = p as u16;
                                        }
                                    }
                                }
                                "anim_mode" => {
                                    if let Some(m) = val.as_str() {
                                        if ["off", "measure", "speed"].contains(&m) {
                                            c.anim_mode = m.to_owned();
                                        }
                                    }
                                }
                                "anim_speed_multiplier" => {
                                    if let Some(f) = val.as_f64() {
                                        if f.is_finite() && f > 0.0 && f <= 1000.0 {
                                            c.anim_speed_multiplier = f as f32;
                                        }
                                    }
                                }
                                "target_frames" => {
                                    if let Some(n) = val.as_u64() {
                                        if n >= 1 {
                                            c.target_frames = n as u32;
                                        }
                                    }
                                }
                                "log_ui_positions" => {
                                    c.log_ui_positions = val.as_bool().unwrap_or(c.log_ui_positions)
                                }
                                "probe_instances" => {
                                    c.probe_instances = val.as_bool().unwrap_or(c.probe_instances)
                                }
                                "uitree_max" => {
                                    if let Some(n) = val.as_u64() {
                                        if n >= 1 {
                                            c.uitree_max = n.min(2000) as u32;
                                        }
                                    }
                                }
                                other => bad.push(other.to_owned()),
                            }
                        }
                    });
                    if !bad.is_empty() {
                        crate::chlog!(warn, "/config 忽略未知字段: {bad:?}");
                    }
                }
                ("200 OK", serde_json::json!({"config": crate::config::get()}).to_string())
            }
        },
        _ => (
            "404 Not Found",
            serde_json::json!({"error": "no such route", "routes": ROUTES}).to_string(),
        ),
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
