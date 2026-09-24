//! Guard 管理的全部 hook。
//!
//! 规则：
//! - 所有目标先查宿主 hook 目录（README 冲突矩阵），只挑**补集**；
//! - 安装一律走 host::hook_guarded，guard 拒绝即跳过并记日志；
//! - hook 内先链回原函数（trampoline），再叠加观测。

use std::os::raw::c_void;

use crate::{chlog, host};

// ============ UnityEngine.CoreModule / Application ============

static mut ORIG_SET_TARGET_FRAME_RATE: usize = 0;

pub unsafe extern "C" fn hook_set_target_frame_rate(this: *mut c_void, rate: i32) {
    chlog!(info, "Application.set_targetFrameRate({rate})");
    let orig: unsafe extern "C" fn(*mut c_void, i32) =
        std::mem::transmute(orig_for(hook_set_target_frame_rate as usize, ORIG_SET_TARGET_FRAME_RATE));
    orig(this, rate);
}

// ============ UnityEngine.CoreModule / PlayableDirector（育成演出取证） ============

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

/// 全量安装。每个目标都可能被 guard 拒绝 —— 拒绝即跳过，绝不影响宿主。
pub fn install_all() {
    let targets: [(u8, Option<&'static str>, &'static str, &'static str, &'static str, &'static str, i32, usize); 3] = [
        (
            0,
            None,
            "UnityEngine.CoreModule",
            "UnityEngine",
            "Application",
            "set_targetFrameRate",
            1,
            hook_set_target_frame_rate as usize,
        ),
        (
            1,
            None,
            "UnityEngine.CoreModule",
            "UnityEngine.Playables",
            "PlayableDirector",
            "Play",
            0,
            hook_play_0 as usize,
        ),
        (
            2,
            None,
            "UnityEngine.CoreModule",
            "UnityEngine.Playables",
            "PlayableDirector",
            "Play",
            1,
            hook_play_1 as usize,
        ),
    ];

    for (slot, sym, asm, ns, cls, mth, args, hook_fn) in targets {
        let Some(orig) = host::get_method_addr(asm, ns, cls, mth, args) else {
            chlog!(warn, "resolve 失败: {cls}::{mth}/{args}");
            continue;
        };
        match host::hook_guarded(orig, hook_fn, sym) {
            Ok(tramp) => {
                unsafe {
                    match slot {
                        0 => ORIG_SET_TARGET_FRAME_RATE = tramp,
                        1 => ORIG_PLAY_0 = tramp,
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
