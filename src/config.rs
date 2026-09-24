//! 配置：<data_dir>/chonggou.json（Hachimi 数据目录，用户可直接编辑）
//!
//! 字段说明：
//! - http_enabled / http_port：诊断 HTTP（18765 被占自动顺延）
//! - anim_mode：off | measure（v1：只观测记录）| speed（提速，v2 落地）
//! - target_frames：育成动画目标帧数（Rue 实测：start 1帧 + loop 1帧 + end 1帧 = 3帧最稳；
//!   总共 1 帧会让程序跳过事件 → 服务器收不到消息 → 必须退大厅救）
//! - anim_speed_multiplier：speed 模式的倍率（v2 使用）
//! - log_ui_positions：UI 位置日志开关（排查"文字位置不对"类问题）

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    pub http_enabled: bool,
    pub http_port: u16,
    pub anim_mode: String,
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

pub fn init(base_dir: Option<String>) {
    *BASE_DIR.lock().unwrap() = base_dir.clone();

    match config_path() {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<Config>(&s) {
                Ok(c) => {
                    crate::chlog!(info, "配置已加载: {}", path.display());
                    *CONFIG.write().unwrap() = c;
                }
                Err(e) => {
                    crate::chlog!(warn, "配置解析失败({e})，使用默认值: {}", path.display());
                }
            },
            Err(_) => {
                // 首次运行：落一份默认配置，方便用户改
                let _ = save();
                crate::chlog!(info, "首次运行，已生成默认配置: {}", path.display());
            }
        },
        None => {
            crate::chlog!(warn, "无数据目录，配置仅内存生效（默认值）");
        }
    }
}

pub fn get() -> Config {
    CONFIG.read().unwrap().clone()
}

pub fn config_path() -> Option<PathBuf> {
    BASE_DIR.lock().unwrap().as_ref().map(|d| PathBuf::from(d).join("chonggou.json"))
}

pub fn save() -> Result<(), String> {
    let path = config_path().ok_or("no base dir")?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = serde_json::to_string_pretty(&*CONFIG.read().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, body).map_err(|e| e.to_string())
}

/// 端口记录文件路径（Agora/外部工具发现用）。
pub fn port_file_path() -> Option<PathBuf> {
    BASE_DIR.lock().unwrap().as_ref().map(|d| PathBuf::from(d).join("chonggou_port.txt"))
}
