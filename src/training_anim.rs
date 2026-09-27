//! 育成内动画 3 帧化（Rue 实测结论：start 1帧 + loop 1帧 + end 1帧 = 3帧最稳）。
//!
//! ⚠️ 语义红线（来自 Rue 的 Windows Steam 实测 + 我们自己的讨论）：
//! - **不能跳过**（skip/删资源）：程序会跳过某个事件 → 服务器收不到消息 →
//!   卡死，必须退大厅才能救；
//! - **总 1 帧也不行**：等同跳过；
//! - **正确姿势**：每个阶段照常完整播，但压缩到 1 帧 —— 事件全部照发。
//!
//! ── v1（0.2.0）= measure ──
//! 只做取证：hook `PlayableDirector::Play` 记录每次演出，外加候选类/命名空间
//! 的「只查址不调用」探测。目的是拿到 cut-in 类的真名再精确落刀。
//!
//! ── v2（0.3.0）= speed 落刀本体 ──
//! **不再等命名空间**：v1 的取证结论是「cut-in 类名在盲区」，但提速落点根本
//! 不需要它 —— 场景里现成的 `UnityEngine.Animator` 就有 `set_speed(float)`，
//! 把速度拉高就是「时间压缩」：整条时间轴照走、事件照发、每个阶段瞬间过，
//! 语义上正是「压缩到 1 帧」而不是「跳过」。
//!
//! 落刀路径完全靠**运行时解析**，不猜：
//! 1. `probe_instances` 总闸（默认关）→ 关着时 `anim_mode=speed` 直接不生效；
//! 2. `Object.FindObjectsOfType(Type,bool)` 拿 Animator 列表；
//! 3. `Animator::set_speed(float)` 解析不到 → 计数上报，不调；
//! 4. 每次调用结果进 `/anim`（applied / skipped / failed）。
//!
//! 节流：`FindObjectsOfType` 要遍历全场景对象，**不能每次 Play 都跑**。
//! 同一时刻只有一个演出在播，按 director 地址去重 + 2 秒冷却。

use once_cell::sync::Lazy;
use serde_json::json;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ASM: &str = "UnityEngine.CoreModule";
const NS: &str = "UnityEngine";
const APPLY_COOLDOWN: Duration = Duration::from_secs(2);

type SetSpeedFn = unsafe extern "C" fn(this: *mut c_void, value: f32);

static PLAYS: AtomicUsize = AtomicUsize::new(0);
static LAST_DIRECTOR: AtomicUsize = AtomicUsize::new(0);
static RESOLVED: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));

static SPEED_APPLIED: AtomicUsize = AtomicUsize::new(0);
static SPEED_SKIPPED: AtomicUsize = AtomicUsize::new(0);
static SPEED_FAILED: AtomicUsize = AtomicUsize::new(0);
static SPEED_PASSES: AtomicUsize = AtomicUsize::new(0);
static LAST_APPLY: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));
static SEEN_DIRECTORS: Lazy<Mutex<Vec<usize>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// PlayableDirector hook 回报。
pub fn note_play(director: usize, overload: u8) {
    PLAYS.fetch_add(1, Ordering::Relaxed);
    LAST_DIRECTOR.store(director, Ordering::Relaxed);
    let _ = overload;
    maybe_apply(director);
}

/// 冷却 + 去重通过才真的落刀。
fn maybe_apply(director: usize) {
    let cfg = crate::config::get();
    if cfg.anim_mode != "speed" {
        return;
    }
    if !cfg.probe_instances {
        SPEED_SKIPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }

    // 同一 director 不重复；换演出了才允许再跑一次扫描
    {
        let mut seen = SEEN_DIRECTORS.lock().unwrap_or_else(|p| p.into_inner());
        if seen.contains(&director) {
            SPEED_SKIPPED.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if seen.len() > 64 {
            seen.clear();
        }
        seen.push(director);
    }

    {
        let mut last = LAST_APPLY.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(t) = *last {
            if t.elapsed() < APPLY_COOLDOWN {
                SPEED_SKIPPED.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        *last = Some(Instant::now());
    }

    apply_speed_now(cfg.anim_speed_multiplier);
}

/// 扫全场景 Animator 并提速。全程可失败，绝不崩。
pub fn apply_speed_now(multiplier: f32) {
    SPEED_PASSES.fetch_add(1, Ordering::Relaxed);

    let Some(set_speed) = crate::host::get_method_addr(ASM, NS, "Animator", "set_speed", 1) else {
        SPEED_FAILED.fetch_add(1, Ordering::Relaxed);
        chlog!(warn, "speed: Animator::set_speed 解析失败，本轮不提速");
        return;
    };
    let f: SetSpeedFn = unsafe { std::mem::transmute(set_speed) };

    let animators = crate::host::find_instances(ASM, NS, "Animator");
    if animators.is_empty() {
        SPEED_FAILED.fetch_add(1, Ordering::Relaxed);
        chlog!(warn, "speed: 未枚举到 Animator 实例（{}）", animators.len());
        return;
    }

    let mut ok = 0usize;
    for a in &animators {
        unsafe { f(*a as *mut c_void, multiplier) };
        ok += 1;
    }
    SPEED_APPLIED.fetch_add(ok, Ordering::Relaxed);
    chlog!(info, "speed: 已对 {ok} 个 Animator 设定 speed={multiplier}");
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
        if crate::host::class_exists(ASM, "UnityEngine.Playables", "PlayableDirector") { "FOUND" } else { "MISS" }
    ));
    for (m, a) in [
        ("Play", 0i32),
        ("Play", 1),
        ("get_duration", 0),
        ("get_time", 0),
        ("get_extrapolationMode", 0),
        ("set_extrapolationMode", 1),
    ] {
        lines.push(probe(ASM, "UnityEngine.Playables", "PlayableDirector", m, a));
    }

    // 2) Animator 提速落点（v2 的 set_speed 就在这里）
    lines.push(probe(ASM, NS, "Animator", "set_speed", 1));
    lines.push(probe(ASM, NS, "Animator", "get_speed", 0));

    // 3) 实例枚举链（v2 依赖，全齐才可能生效）
    for sym in ["il2cpp_class_get_type", "il2cpp_resolve_symbol"] {
        lines.push(format!(
            "sym {sym}: {}",
            match crate::host::resolve_symbol(sym) {
                Some(p) => format!("OK @ {p:#x}"),
                None => "MISS".to_owned(),
            }
        ));
    }
    lines.push(probe(ASM, NS, "Object", "FindObjectsOfType", 2));

    // 4) /uitree 依赖
    for (c, m, a) in [
        ("Object", "get_name", 0),
        ("RectTransform", "get_anchoredPosition", 0),
        ("RectTransform", "get_sizeDelta", 0),
        ("Transform", "get_localScale", 0),
        ("Transform", "get_parent", 0),
    ] {
        lines.push(probe(ASM, NS, c, m, a));
    }

    // 5) 育成 cut-in 候选（命名空间盲区 —— 命中与否全量记录，留档）
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

    *RESOLVED.lock().unwrap_or_else(|p| p.into_inner()) = lines;
    chlog!(info, "解析表已刷新（/anim 可读）");

    // 取证表建好，顺手把 /uitree 抓一次（总闸关着时只是记一条降级说明）
    crate::ui_probe::refresh();
}

/// /anim 端点快照。
pub fn snapshot_json() -> String {
    let resolved = RESOLVED.lock().unwrap_or_else(|p| p.into_inner());
    let cfg = crate::config::get();
    json!({
        "mode": cfg.anim_mode,
        "target_frames": cfg.target_frames,
        "speed_multiplier": cfg.anim_speed_multiplier,
        "probe_instances": cfg.probe_instances,
        "note": "speed 模式需要 probe_instances=true；语义是时间压缩（事件照发），不是 skip",
        "plays": PLAYS.load(Ordering::Relaxed),
        "last_director": format!("{:#x}", LAST_DIRECTOR.load(Ordering::Relaxed)),
        "speed": {
            "passes": SPEED_PASSES.load(Ordering::Relaxed),
            "applied": SPEED_APPLIED.load(Ordering::Relaxed),
            "skipped": SPEED_SKIPPED.load(Ordering::Relaxed),
            "failed": SPEED_FAILED.load(Ordering::Relaxed),
        },
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
        assert!(s.contains("speed"));
        assert!(s.contains("probe_instances"));
    }

    #[test]
    fn measure_mode_never_applies() {
        crate::config::update(|c| {
            c.anim_mode = "measure".to_owned();
            c.probe_instances = true;
        });
        let before = SPEED_PASSES.load(Ordering::Relaxed);
        maybe_apply(0xDEAD);
        assert_eq!(SPEED_PASSES.load(Ordering::Relaxed), before);
    }

    #[test]
    fn gate_closed_never_scans() {
        crate::config::update(|c| {
            c.anim_mode = "speed".to_owned();
            c.probe_instances = false;
        });
        SEEN_DIRECTORS.lock().unwrap_or_else(|p| p.into_inner()).clear();
        *LAST_APPLY.lock().unwrap_or_else(|p| p.into_inner()) = None;
        let before = SPEED_PASSES.load(Ordering::Relaxed);
        maybe_apply(0xBEEF);
        assert_eq!(SPEED_PASSES.load(Ordering::Relaxed), before);
    }
}
