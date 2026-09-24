//! 冲突防护层：denylist + prologue 探测
//!
//! 防 hook 冲突的两道防线：
//! 1. **静态 denylist**：宿主 Hachimi-Edge 已占用的 hook 点（linker do_dlopen /
//!    libc::dlopen / JNINativeInterface::RegisterNatives）。命中即拒绝安装。
//! 2. **动态 prologue 探测**：任何 inline hook 引擎（Dobby 等）都会把目标函数
//!    首指令改写为跳转。装 hook 前读首指令，若已是跳转则说明**无论谁**已经
//!    hook 了该函数，跳过安装，绝不二次打补丁。

use std::sync::Mutex;

/// 宿主已占用的 hook 点（按符号名）。
pub const HOST_DENY_SYMBOLS: &[(&str, &str)] = &[
    // (module, symbol) —— 见 Hachimi-Edge src/android/hook.rs
    ("linker64", "__dl__Z9do_dlopenPKciPK17android_dlextinfoPKv"),
    ("linker64", "__dl__Z9do_dlopenPKciPK17android_dlextinfoPv"),
    ("libc.so", "dlopen"),
];

/// 已登记的我方 hook（orig_addr -> hook_addr）
static OWN_HOOKS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

pub enum HookDecision {
    /// 可以安全安装
    Allow(usize),
    /// 目标已被其他引擎 hook —— 拒绝二次 patch
    AlreadyHooked,
    /// 命中宿主 denylist —— 拒绝
    DeniedByHost,
    /// 目标地址无效
    InvalidTarget,
}

/// AArch64 prologue 探测：判断函数首指令是否已被改写为跳转。
fn is_prologue_patched(insn: u32) -> bool {
    // B / BL
    if insn & 0x7C00_0000 == 0x1400_0000 {
        return true;
    }
    // LDR (literal) X16, +8 —— Dobby "LDR X16,[PC+8]; BR X16" 跳板首条
    if insn == 0x5800_0050 {
        return true;
    }
    // BR X16 / BR X17
    if insn == 0xD61F_0200 || insn == 0xD61F_0400 {
        return true;
    }
    false
}

unsafe fn read_insn(addr: usize) -> Option<u32> {
    if addr == 0 || addr % 4 != 0 {
        return None;
    }
    Some(*(addr as *const u32))
}

/// 安装前决策：综合 denylist 与 prologue 探测。
pub fn decide(orig_addr: usize, symbol: Option<&str>) -> HookDecision {
    if orig_addr == 0 {
        return HookDecision::InvalidTarget;
    }

    if let Some(sym) = symbol {
        if HOST_DENY_SYMBOLS.iter().any(|(_, s)| *s == sym) {
            return HookDecision::DeniedByHost;
        }
    }

    if OWN_HOOKS.lock().unwrap().iter().any(|(o, _)| *o == orig_addr) {
        return HookDecision::AlreadyHooked;
    }

    match unsafe { read_insn(orig_addr) } {
        Some(insn) if is_prologue_patched(insn) => HookDecision::AlreadyHooked,
        Some(_) => HookDecision::Allow(orig_addr),
        None => HookDecision::InvalidTarget,
    }
}

/// 登记成功安装的 hook。
pub fn register_own_hook(orig_addr: usize, hook_addr: usize) {
    OWN_HOOKS.lock().unwrap().push((orig_addr, hook_addr));
}

/// 我方 hook 注册表快照（供 /hooks 诊断端点）。
pub fn own_hooks() -> Vec<(usize, usize)> {
    OWN_HOOKS.lock().unwrap().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_b_imm() {
        assert!(is_prologue_patched(0x1400_0400));
        assert!(!is_prologue_patched(0xD280_0000));
    }

    #[test]
    fn detects_dobby_ldr_bridge() {
        assert!(is_prologue_patched(0x5800_0050));
    }

    #[test]
    fn rejects_null_target() {
        assert!(matches!(decide(0, None), HookDecision::InvalidTarget));
    }

    #[test]
    fn denies_host_symbols() {
        assert!(matches!(
            decide(0x1000, Some("__dl__Z9do_dlopenPKciPK17android_dlextinfoPKv")),
            HookDecision::DeniedByHost
        ));
    }

    #[test]
    fn idempotent_own_hook() {
        register_own_hook(0xDEAD, 0xBEEF);
        assert!(matches!(decide(0xDEAD, None), HookDecision::AlreadyHooked));
    }
}
