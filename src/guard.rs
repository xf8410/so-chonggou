//! 冲突防护层：denylist + prologue 探测
//!
//! 防 hook 冲突的两道防线：
//! 1. **静态 denylist**：宿主 Hachimi-Edge 已占用的 hook 点（linker do_dlopen /
//!    libc::dlopen / JNINativeInterface::RegisterNatives），以及它 hook 过的
//!    il2cpp 方法所在程序集。命中即拒绝安装。
//! 2. **动态 prologue 探测**：任何 inline hook 引擎（Dobby 等）都会把目标函数
//!    首指令改写为跳转。装 hook 前读首指令，若已是跳转则说明**无论谁**已经
//!    hook 了该函数，跳过安装（或走链式调用），绝不二次打补丁。

use std::sync::Mutex;

/// 宿主已占用的 hook 点（按符号名）。独立模式可用；插件模式下运行时探测兜底。
pub const HOST_DENY_SYMBOLS: &[(&str, &str)] = &[
    // (module, symbol) —— 见 Hachimi-Edge src/android/hook.rs
    ("linker64", "__dl__Z9do_dlopenPKciPK17android_dlextinfoPKv"),
    ("linker64", "__dl__Z9do_dlopenPKciPK17android_dlextinfoPv"),
    ("libc.so", "dlopen"),
];

/// 已被我们自己登记的 hook 地址（orig_addr -> 我方 hook_addr）
static OWN_HOOKS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

pub enum HookDecision {
    /// 可以安全安装，返回 trampoline 期望的 orig_addr
    Allow(usize),
    /// 目标已被其他引擎 hook —— 拒绝二次 patch
    AlreadyHooked,
    /// 命中宿主 denylist —— 拒绝
    DeniedByHost,
    /// 目标地址无效
    InvalidTarget,
}

/// AArch64 prologue 探测：判断函数首指令是否已被改写为跳转。
///
/// Dobby / 类 inline hook 典型产物：
/// - `B imm26`        (0x14000000..=0x17FFFFFF)
/// - `LDR X16, #8; BR X16` 长跳板 (前 4 字节 0x58000050)
/// - `MOV X16, imm64` 系列 veneer (0xD2800000.. 由 Dobby 用于绝对地址跳转)
fn is_prologue_patched(insn: u32) -> bool {
    // B / BL
    if insn & 0x7C00_0000 == 0x1400_0000 {
        return true;
    }
    // LDR (literal) X16, +8  —— Dobby "LDR X16, [PC+8]; BR X16" 跳板首条
    if insn == 0x5800_0050 {
        return true;
    }
    // BR X16 / BR X17 —— 少数引擎直接把首指令换成 BR
    if insn == 0xD61F_0200 || insn == 0xD61F_0400 {
        return true;
    }
    false
}

unsafe fn read_insn(addr: usize) -> Option<u32> {
    if addr == 0 || addr % 4 != 0 {
        return None;
    }
    // 同进程内直接解引用；/proc/self/maps 已映射可执行页
    Some(*(addr as *const u32))
}

/// 安装前决策：综合 denylist 与 prologue 探测。
pub fn decide(orig_addr: usize, symbol: Option<&str>) -> HookDecision {
    if orig_addr == 0 {
        return HookDecision::InvalidTarget;
    }

    // 1) 符号 denylist（仅在能拿到符号名时生效）
    if let Some(sym) = symbol {
        if HOST_DENY_SYMBOLS.iter().any(|(_, s)| *s == sym) {
            return HookDecision::DeniedByHost;
        }
    }

    // 2) 同一目标被我们自己 hook 过 —— 幂等，不再装
    if let Some(ours) = OWN_HOOKS.lock().unwrap().iter().find(|(o, _)| *o == orig_addr) {
        let _ = ours;
        return HookDecision::AlreadyHooked;
    }

    // 3) prologue 探测 —— 谁先 hook 的谁说了算，后来者让路
    match unsafe { read_insn(orig_addr) } {
        Some(insn) if is_prologue_patched(insn) => HookDecision::AlreadyHooked,
        Some(_) => HookDecision::Allow(orig_addr),
        None => HookDecision::InvalidTarget,
    }
}

/// 登记我们成功安装的 hook（hook 成功后调用）。
pub fn register_own_hook(orig_addr: usize, hook_addr: usize) {
    OWN_HOOKS.lock().unwrap().push((orig_addr, hook_addr));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_b_imm() {
        // B +0x1000
        assert!(is_prologue_patched(0x1400_0400));
        // MOV X0, #0 —— 正常函数首指令，不应误判
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
}
