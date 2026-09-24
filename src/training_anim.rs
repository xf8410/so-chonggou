//! 育成内动画 3 帧化（Rue 实测结论：start 1帧 + loop 1帧 + end 1帧 = 3帧最稳）。
//!
//! ⚠️ 语义红线（来自 Rue 的 Windows Steam 实测 + 我们自己的讨论）：
//! - **不能跳过**（skip/删资源）：程序会跳过某个事件 → 服务器收不到消息 →
//!   卡死，必须退大厅才能救；
//! - **总 1 帧也不行**：等同跳过；
//! - **正确姿势**：每个阶段照常完整播，但压缩到 1 帧 —— 事件全部照发。
//!
//! v1 策略 = **measure（取证）**：
//! 育成 cut-in 的确切类名/命名空间在 IL2CPP 里是盲区（il2cpp_get_class 需要
//! 精确命名空间）。所以 v1 只做两件事：
//! 1. hook `PlayableDirector::Play`（宿主未占用，guard 已核）记录每一次演出
//!    播放 —— 这给出"育成里到底谁在播"的实证；
//! 2. 对候选类/命名空间做**只查址不调用**的探测（resolve），命中/落空全量
//!    写进 /anim 端点 —— 下一轮装机跑一局育成，日志直接告诉我们真名，
//!    v2 就能把提速精确落到 `Animator::set_speed` / director graph 上。

use once_cell::sync::Lazy;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

static PLAYS: AtomicUsize = AtomicUsize::new(0);
static LAST_DIRECTOR: AtomicUsize = AtomicUsize::new(0);
static RESOLVED: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// PlayableDirector hook 回报。
pub fn note_play(director: usize, overload: u8) {
    PLAYS.fetch_add(1, Ordering::Relaxed);
    LAST_DIRECTOR.store(director, Ordering::Relaxed);
    let _ = overload;
}

/// 只查址不调用 —— 探测安全。
fn probe(assembly: &str, ns: &str, class: &str, method: &str, args: i32) -> String {
    match crate::host::get_method_addr(assembly, ns, class, method, args) {
        Some(p) => format!("OK   {ns}.{class}::{method}/{args} @ {p:#x}"),
        None => format!("MISS {ns}.{class}::{method}/{args}"),
    }
}

/// 候选解析 + 取证表刷新（游戏初始化后调用一次）。
pub fn resolve_candidates() {
    let mut lines = Vec::new();

    // 1) PlayableDirector（演出主战场；宿主未 hook，无冲突面）
    lines.push(format!(
        "class: {}",
        if crate::host::class_exists("UnityEngine.CoreModule", "UnityEngine.Playables", "PlayableDirector") { "FOUND" } else { "MISS" }
    ));
    for (m, a) in [
        ("Play", 0i32),
        ("Play", 1),
        ("get_duration", 0),
        ("get_time", 0),
        ("get_extrapolationMode", 0),
        ("set_extrapolationMode", 1),
    ] {
        lines.push(probe("UnityEngine.CoreModule", "UnityEngine.Playables", "PlayableDirector", m, a));
    }

    // 2) Animator 提速落点（v2 的 set_speed 从这里取址）
    lines.push(probe("UnityEngine.CoreModule", "UnityEngine", "Animator", "set_speed", 1));
    lines.push(probe("UnityEngine.CoreModule", "UnityEngine", "Animator", "get_speed", 0));

    // 3) 育成 cut-in 候选（命名空间盲区 —— 命中与否全量记录）
    for ns in ["Gallop", "Gallop.SingleMode", "Gallop.CutIn", "Gallop.Cute", "Cute", ""] {
        for class in [
            "SingleModeTrainingCutInHelper",
            "SingleModeCutInController",
            "TrainingCutInController",
            "CutInController",
        ] {
            if crate::host::class_exists("Assembly-CSharp", ns, class) {
                lines.push(format!("FOUND class Assembly-CSharp/{ns}.{class}"));
                for m in ["SkipRuntime", "Init", "Play", "get_state", "set_state"] {
                    lines.push(probe("Assembly-CSharp", ns, class, m, 0));
                }
            }
        }
    }

    *RESOLVED.lock().unwrap() = lines;
    crate::chlog!(info, "3帧化取证表已刷新（/anim 端点可读）");
}

/// /anim 端点快照。
pub fn snapshot_json() -> String {
    let resolved = RESOLVED.lock().unwrap();
    let cfg = crate::config::get();
    json!({
        "mode": cfg.anim_mode,
        "target_frames": cfg.target_frames,
        "note": "v1=measure; v2 在取证表定谳命名空间后落地 speed",
        "plays": PLAYS.load(Ordering::Relaxed),
        "last_director": format!("{:#x}", LAST_DIRECTOR.load(Ordering::Relaxed)),
        "resolved": *resolved,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_has_fields() {
        let s = snapshot_json();
        assert!(s.contains("target_frames"));
        assert!(s.contains("plays"));
    }
}
