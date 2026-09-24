//! Guard 管理的全部 hook。
//!
//! 三道闸，一个都不能少：
//! 1. **denylist 类清单**（src/denylist.rs）—— 宿主 hook 过的类直接拒绝；
//! 2. **prologue 探测**（guard::decide）—— 目标方法首指令已是跳板就让路；
//! 3. 安装一律走 host::hook_guarded，任何拒绝都记日志。

use std::os::raw::c_void;

use crate::host;

// ============ UnityEngine.CoreModule / PlayableDirector（育成演出取证） ============
//
// 选它的原因：PlayableDirector **不在宿主 hook 清单**（实查 Hachimi-Edge
// src/il2cpp/hook/**），是干净目标；且它是育成 cut-in / 演出的播放入口，
// 是 3 帧化（docs 见 training_anim.rs）的观测点。

static mut ORIG_PLAY_0: usize = 0;
static mut ORIG_PLAY_1: usize = 0;

type Play0 = unsafe extern "C" fn(this: *mut c_void);
type Play1 = unsafe extern "C" fn(this: *mut c_void, asset: *mut c_void);

pub unsafe extern "C" fn hook_play_0(this: *mut c_void) {
    chlog!(info, "PlayableDirector.Play(this={:p})", this);
    crate::training_anim::note_play(this as usize, 0);
    let orig: Play0 = std::mem::transmute(orig_for(hook_play_0 as usize, ORIG_PLAY_0));
    orig(this);
}

pub unsafe extern "C" fn hook_play_1(this: *mut c_void, asset: *mut c_void) {
    chlog!(info, "PlayableDirector.Play(asset, this={:p})", this);
    crate::training_anim::note_play(this as usize, 1);
    let orig: Play1 = std::mem::transmute(orig_for(hook_play_1 as usize, ORIG_PLAY_1));
    orig(this, asset);
}

/// 原函数地址：优先宿主 trampoline，回退安装时保存的值。
fn orig_for(hook_fn: usize, fallback: usize) -> usize {
    match host::trampoline(hook_fn) {
        Some(t) if t != 0 => t,
        _ => fallback,
    }
}

/// 全量安装。每个目标都可能被闸门拒绝 —— 拒绝即跳过，绝不影响宿主。
pub fn install_all() {
    let targets: [(Option<&'static str>, &'static str, &'static str, &'static str, &'static str, i32, usize); 2] = [
        (
            None,
            "UnityEngine.CoreModule",
            "UnityEngine.Playables",
            "PlayableDirector",
            "Play",
            0,
            hook_play_0 as usize,
        ),
        (
            None,
            "UnityEngine.CoreModule",
            "UnityEngine.Playables",
            "PlayableDirector",
            "Play",
            1,
            hook_play_1 as usize,
        ),
    ];

    for (sym, asm, ns, cls, mth, args, hook_fn) in targets {
        // 第 0 道闸：类级 denylist
        if crate::denylist::class_denied(cls) {
            chlog!(warn, "拒绝 {cls}::{mth}：类在宿主 hook 清单（docs/CONFLICTS.md）");
            continue;
        }
        let Some(orig) = host::get_method_addr(asm, ns, cls, mth, args) else {
            chlog!(warn, "resolve 失败: {cls}::{mth}/{args}");
            continue;
        };
        // 第 2 道闸：prologue 探测（在 hook_guarded 内）
        match host::hook_guarded(orig, hook_fn, sym) {
            Ok(tramp) => {
                unsafe {
                    match hook_fn {
                        x if x == hook_play_0 as usize => ORIG_PLAY_0 = tramp,
                        _ => ORIG_PLAY_1 = tramp,
                    }
                }
                chlog!(info, "hooked {cls}::{mth}/{args} orig={orig:#x} tramp={tramp:#x}");
            }
            Err(()) => {
                chlog!(warn, "跳过 {cls}::{mth}/{args}（guard: 已被占用或拒绝）");
            }
        }
    }

    crate::training_anim::resolve_candidates();
}
