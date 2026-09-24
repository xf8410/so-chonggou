//! 示例 hook 集：演示“安全安装 + 链式调用”的标准姿势。
//!
//! 规则：
//! - 所有目标必须来自宿主 hook 点的**补集**（见 README「冲突矩阵」）；
//! - 安装一律走 host::hook_guarded，让 guard 决定装 / 不装；
//! - hook 内先调 trampoline 链回原逻辑，再叠加自己的行为。

use std::os::raw::c_void;
use crate::host;

/// 示例：跟踪 Application 可见性变化（宿主未 hook 此函数）。
/// sig: void Application::set_targetFrameRate(int)
static mut ORIG_SET_TARGET_FRAME_RATE: usize = 0;

pub unsafe extern "C" fn hook_set_target_frame_rate(this: *mut c_void, rate: i32) {
    let orig: extern "C" fn(*mut c_void, i32) =
        std::mem::transmute(host::trampoline(hook_set_target_frame_rate as usize).unwrap_or(ORIG_SET_TARGET_FRAME_RATE));
    host::host_log(3, "chonggou", &format!("set_targetFrameRate({rate})"));
    orig(this, rate);
}

/// 启动时安装我们全部的 hook。每个都可能被 guard 拒绝 —— 拒绝即跳过，不影响宿主。
pub fn install_all() {
    let candidates: &[(Option<&str>, &str, &str, &str, i32, usize)] = &[
        // (模块符号, assembly, namespace, class, method, args, hook fn)
        (
            None,
            "UnityEngine.CoreModule",
            "UnityEngine",
            "Application",
            "set_targetFrameRate",
            1,
            hook_set_target_frame_rate as usize,
        ),
        // 新增 hook 只需在这里追加；guard 会自动防冲突。
    ];

    for (sym, asm, ns, cls, mth, args, hook_fn) in candidates {
        let Some(orig) = host::get_method_addr(asm, ns, cls, mth, *args) else {
            host::host_log(2, "chonggou", &format!("resolve failed: {cls}::{mth}"));
            continue;
        };
        unsafe {
            ORIG_SET_TARGET_FRAME_RATE = orig; // 示例仅单目标；多目标请改用 map
        }
        match host::hook_guarded(orig, *hook_fn, *sym) {
            Ok(_tramp) => host::host_log(3, "chonggou", &format!("hooked {cls}::{mth}")),
            Err(()) => host::host_log(2, "chonggou", &format!("skipped {cls}::{mth} (conflict guard)")),
        }
    }
}
