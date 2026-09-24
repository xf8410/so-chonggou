//! 配置解析（三层来源，全部失败也不崩）：
//!
//! 1. **外部媒体目录**（优先）：`/sdcard/Android/media/<包名>/hachimi/chonggou.json`
//!    —— 文件管理器可直接编辑。注意：宿主开了「使用内部文件目录」后宿主目录
//!    在 /data/data 下，外部反而摸不到；反之在老设备上外部才可写。所以两个位置都试。
//! 2. **宿主数据目录**：Hachimi 通过 `hachimi_get_base_dir` 给的目录。
//! 3. **内存默认值**：连文件系统都不可用时，配置照常工作（只是不可持久化）。
//!
//! 防闪退纪律：**本模块任何 fs / 锁操作都不得 panic** —— 所有的 lock 用
//! poison 恢复（`unwrap_or_else(|p| p.into_inner())`），所有 fs 都走 Result。

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    pub http_enabled: bool,
    pub http_port: u16,
    pub anim_mode: String, // off | measure | speed
    pub anim_speed_multiplier: f32,
    pub target_frames: u32,
    pub log_ui_positions: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            http_enabled: true,
            http_port: 18765,
            anim_mode: "measure".to_owned(),
            anim_speed_multiplier: 20.0,
            target_frames: 3,
            log_ui_positions: false,
        }
    }
}

static CONFIG: Lazy<RwLock<Config>> = Lazy::new(|| RwLock::new(Config::default()));
static BASE_DIR: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
static EXTERNAL_DIR: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

pub fn init(base_dir: Option<String>) {
    *BASE_DIR.lock().unwrap_or_else(|p| p.into_inner()) = base_dir.clone();
    let external = external_media_dir();
    *EXTERNAL_DIR.lock().unwrap_or_else(|p| p.into_inner()) = external.clone();

    let mut cfg = Config::default();
    let mut source = "";

    // 1) 外部媒体目录（用户可直接编辑）
    if source.is_empty() {
        if let Some(ext) = &external {
            let p = PathBuf::from(ext).join("chonggou.json");
            match std::fs::read_to_string(&p) {
                Ok(s) => match serde_json::from_str::<Config>(&s) {
                    Ok(c) => {
                        cfg = c;
                        source = "external";
                    }
                    Err(e) => crate::chlog!(warn, "外部配置解析失败({e}): {}", p.display()),
                },
                Err(_) => { /* 不存在/不可读 —— 正常降级 */ }
            }
        }
    }

    // 2) 宿主数据目录
    if source.is_empty() {
        if let Some(path) = host_config_path() {
            if let Ok(s) = std::fs::read_to_string(&path) {
                match serde_json::from_str::<Config>(&s) {
                    Ok(c) => {
                        cfg = c;
                        source = "host";
                    }
                    Err(e) => crate::chlog!(warn, "宿主目录配置解析失败({e}): {}", path.display()),
                }
            }
        }
    }

    if !source.is_empty() {
        *CONFIG.write().unwrap_or_else(|p| p.into_inner()) = cfg;
        crate::chlog!(info, "配置已加载（来源 {source}）");
    } else {
        // 首次运行：默认配置尽量落一份（外部优先，宿主兜底），失败也不影响运行
        match save() {
            Ok(p) => crate::chlog!(info, "首次运行，默认配置已生成: {}", p.display()),
            Err(e) => crate::chlog!(warn, "配置仅内存生效（写入失败: {e}）"),
        }
    }
}

pub fn get() -> Config {
    CONFIG.read().unwrap_or_else(|p| p.into_inner()).clone()
}

/// 宿主目录里的配置文件路径。
pub fn host_config_path() -> Option<PathBuf> {
    BASE_DIR
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|d| PathBuf::from(d).join("chonggou.json"))
}

/// 外部媒体目录（若可解析）。
pub fn external_dir() -> Option<String> {
    EXTERNAL_DIR.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// 保存：外部优先，宿主兜底，返回实际写成功的路径。
pub fn save() -> Result<PathBuf, String> {
    let body = serde_json::to_string_pretty(&*CONFIG.read().unwrap_or_else(|p| p.into_inner()))
        .map_err(|e| e.to_string())?;

    let mut last_err = String::new();
    if let Some(ext) = external_dir() {
        let p = PathBuf::from(&ext).join("chonggou.json");
        if let Some(par) = p.parent() {
            let _ = std::fs::create_dir_all(par);
        }
        match std::fs::write(&p, &body) {
            Ok(_) => return Ok(p),
            Err(e) => last_err = format!("external({}): {e}", p.display()),
        }
    }
    if let Some(p) = host_config_path() {
        if let Some(par) = p.parent() {
            let _ = std::fs::create_dir_all(par);
        }
        match std::fs::write(&p, &body) {
            Ok(_) => return Ok(p),
            Err(e) => {
                if !last_err.is_empty() {
                    last_err.push_str("; ");
                }
                last_err.push_str(&format!("host({}): {e}", p.display()));
            }
        }
    }
    Err(if last_err.is_empty() { "no writable dir".to_owned() } else { last_err })
}

/// 端口记录文件的全部候选路径（全部 best-effort 写入）。
pub fn port_file_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(ext) = external_dir() {
        v.push(PathBuf::from(ext).join("chonggou_port.txt"));
    }
    if let Some(base) = BASE_DIR.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        v.push(PathBuf::from(base).join("chonggou_port.txt"));
    }
    v
}

/// 从 /proc/self/cmdline 第一段取包名。
pub fn parse_package(cmdline: &str) -> Option<String> {
    let pkg = cmdline.split('\0').next()?.trim();
    if pkg.is_empty() {
        None
    } else {
        Some(pkg.to_owned())
    }
}

/// 外部媒体目录：`/sdcard/Android/media/<包名>/hachimi`。
/// 解析失败返回 None —— 调用方照常降级，绝不 panic。
pub fn external_media_dir() -> Option<String> {
    let cmdline = std::fs::read_to_string("/proc/self/cmdline").ok()?;
    let pkg = parse_package(&cmdline)?;
    Some(format!("/sdcard/Android/media/{pkg}/hachimi"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_package_name() {
        assert_eq!(
            parse_package("jp.co.cygames.umamusume\0--arg\0"),
            Some("jp.co.cygames.umamusume".to_owned())
        );
        assert_eq!(parse_package(""), None);
        assert_eq!(parse_package("\0rest"), None);
    }

    #[test]
    fn default_config_sane() {
        let c = Config::default();
        assert_eq!(c.http_port, 18765);
        assert_eq!(c.target_frames, 3);
        assert!(c.http_enabled);
    }

    #[test]
    fn config_json_roundtrip() {
        let c = Config::default();
        let s = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&s).unwrap();
        assert_eq!(back.http_port, c.http_port);
        // 缺字段也能解析（serde default）
        let partial: Config = serde_json::from_str("{\"http_port\":18766}").unwrap();
        assert_eq!(partial.http_port, 18766);
        assert_eq!(partial.target_frames, 3);
    }
}
