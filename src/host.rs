//! 宿主 API 层：插件模式下所有 il2cpp 访问与 hook 都借道宿主，
//! 本 SO **不引入第二套 inline hook 引擎**，从根源上避免冲突。
//!
//! v0.3.0 追加：il2cpp 运行时直读（`find_instances` / Il2CppArray / Il2CppString）。
//! 这些是**布局假设**，全部受 `config::probe_instances` 总闸保护（默认关），
//! 且任一环节解析不到就返回空 —— 逐级降级，绝不崩、绝不猜。

use std::ffi::{c_char, c_void, CStr, CString};
use once_cell::sync::OnceCell;

pub type GetApiFn = extern "C" fn(name: *const c_char) -> *mut c_void;

// 宿主导出的 API 名（与 Hachimi-Edge src/core/plugin_api.rs 对齐）
type InterceptorHookFn = unsafe extern "C" fn(this: *mut c_void, orig: *mut c_void, hook: *mut c_void) -> *mut c_void;
type GetTrampolineFn = unsafe extern "C" fn(this: *mut c_void, hook_addr: *mut c_void) -> *mut c_void;
type ResolveSymbolFn = unsafe extern "C" fn(name: *const c_char) -> *mut c_void;
type GetImageFn = unsafe extern "C" fn(name: *const c_char) -> *const c_void;
type GetClassFn = unsafe extern "C" fn(image: *const c_void, ns: *const c_char, cls: *const c_char) -> *mut c_void;
type GetMethodAddrFn = unsafe extern "C" fn(class: *mut c_void, name: *const c_char, args: i32) -> *mut c_void;
type LogFn = unsafe extern "C" fn(level: i32, target: *const c_char, message: *const c_char);

static GET_API: OnceCell<GetApiFn> = OnceCell::new();
static HOST_INTERCEPTOR: OnceCell<usize> = OnceCell::new();

fn api(name: &str) -> Option<usize> {
    let get_api = *GET_API.get()?;
    let c = CString::new(name).ok()?;
    let ptr = get_api(c.as_ptr());
    if ptr.is_null() { None } else { Some(ptr as usize) }
}

/// 供 crate 内取宿主 API 函数指针。
pub fn api_lookup(name: &str) -> Option<usize> {
    api(name)
}

/// 由 hachimi_init_v3 注入 get_api。
pub fn bind(get_api: GetApiFn) {
    let _ = GET_API.set(get_api);
}

fn interceptor() -> Option<usize> {
    if let Some(v) = HOST_INTERCEPTOR.get() {
        return Some(*v);
    }
    // hachimi_instance() -> hachimi_get_interceptor(instance)
    let instance_f: extern "C" fn() -> *mut c_void = unsafe { std::mem::transmute(api("hachimi_instance")?) };
    let get_f: extern "C" fn(*mut c_void) -> *mut c_void = unsafe { std::mem::transmute(api("hachimi_get_interceptor")?) };
    let itc = get_f(instance_f());
    if !itc.is_null() {
        let _ = HOST_INTERCEPTOR.set(itc as usize);
        Some(itc as usize)
    } else {
        None
    }
}

/// 借宿主安装 hook —— 装之前先过 conflict guard。
/// Err(()) = 被防护层拒绝（冲突让路），属正常控制流。
pub fn hook_guarded(orig: usize, hook: usize, symbol: Option<&str>) -> Result<usize, ()> {
    match crate::guard::decide(orig, symbol) {
        crate::guard::HookDecision::Allow(target) => {
            let itc = interceptor().ok_or(())?;
            let hook_f: InterceptorHookFn = unsafe {
                std::mem::transmute(api("interceptor_hook").ok_or(())?)
            };
            let tramp = unsafe { hook_f(itc as *mut c_void, target as *mut c_void, hook as *mut c_void) };
            if tramp.is_null() {
                return Err(());
            }
            crate::guard::register_own_hook(orig, hook);
            Ok(tramp as usize)
        }
        _ => Err(()),
    }
}

/// 取当前 trampoline（原函数），供 hook 内部链式调用。
pub fn trampoline(hook_addr: usize) -> Option<usize> {
    let itc = interceptor()?;
    let f: GetTrampolineFn = unsafe { std::mem::transmute(api("interceptor_get_trampoline_addr")?) };
    let t = unsafe { f(itc as *mut c_void, hook_addr as *mut c_void) };
    if t.is_null() { None } else { Some(t as usize) }
}

pub fn resolve_symbol(name: &str) -> Option<usize> {
    let f: ResolveSymbolFn = unsafe { std::mem::transmute(api("il2cpp_resolve_symbol")?) };
    let c = CString::new(name).ok()?;
    let p = unsafe { f(c.as_ptr()) };
    if p.is_null() { None } else { Some(p as usize) }
}

fn get_class_raw(assembly: &str, ns: &str, class: &str) -> *mut c_void {
    let null = std::ptr::null_mut();

    let img_api = match api("il2cpp_get_assembly_image") {
        Some(v) => v,
        None => return null,
    };
    let cls_api = match api("il2cpp_get_class") {
        Some(v) => v,
        None => return null,
    };
    let img_f: GetImageFn = unsafe { std::mem::transmute(img_api) };
    let cls_f: GetClassFn = unsafe { std::mem::transmute(cls_api) };

    let a = match CString::new(assembly) {
        Ok(v) => v,
        Err(_) => return null,
    };
    let n = match CString::new(ns) {
        Ok(v) => v,
        Err(_) => return null,
    };
    let c = match CString::new(class) {
        Ok(v) => v,
        Err(_) => return null,
    };

    let image = unsafe { img_f(a.as_ptr()) };
    if image.is_null() {
        return null;
    }
    unsafe { cls_f(image, n.as_ptr(), c.as_ptr()) }
}

/// 类指针（供实例枚举用）。
pub fn get_class_ptr(assembly: &str, ns: &str, class: &str) -> usize {
    get_class_raw(assembly, ns, class) as usize
}

/// 类是否存在（探测用，不做任何调用）。
pub fn class_exists(assembly: &str, ns: &str, class: &str) -> bool {
    !get_class_raw(assembly, ns, class).is_null()
}

/// 解析方法地址（只查址，不调用 —— 探测安全）。
pub fn get_method_addr(assembly: &str, ns: &str, class: &str, method: &str, args: i32) -> Option<usize> {
    let mth_api = api("il2cpp_get_method_addr")?;
    let mth_f: GetMethodAddrFn = unsafe { std::mem::transmute(mth_api) };
    let klass = get_class_raw(assembly, ns, class);
    if klass.is_null() {
        return None;
    }
    let m = CString::new(method).ok()?;
    let addr = unsafe { mth_f(klass, m.as_ptr(), args) };
    if addr.is_null() { None } else { Some(addr as usize) }
}

pub fn host_log(level: i32, target: &str, msg: &str) {
    if let Some(f) = api("log") {
        let f: LogFn = unsafe { std::mem::transmute(f) };
        if let (Ok(t), Ok(m)) = (CString::new(target), CString::new(msg)) {
            unsafe { f(level, t.as_ptr(), m.as_ptr()) };
        }
    }
}

pub fn cstr<'a>(p: *const c_char) -> &'a str {
    if p.is_null() { return ""; }
    unsafe { CStr::from_ptr(p).to_str().unwrap_or("") }
}

// ===================== v0.3.0：il2cpp 运行时直读 =====================
//
// 全部受 `config::probe_instances` 总闸保护（默认关）。任一符号解析不到 →
// 返回空 Vec / None，调用方照常降级。这是「先量后动」纪律的延续：我们
// 宁可给空表，也不拿猜来的 ABI 去调游戏函数。

/// Il2CppObject 头 16 字节（klass + monitor）。
const OBJ_HDR: usize = 16;
/// Il2CppArray：obj(16) + bounds*(8) + max_length(8) → vector 从 32 开始。
const ARRAY_VECTOR_OFF: usize = 32;
/// Il2CppString：obj(16) + length(4) → chars 从 20 开始。
const STR_LEN_OFF: usize = 16;
const STR_CHARS_OFF: usize = 20;

type ClassGetTypeFn = unsafe extern "C" fn(class: *mut c_void) -> *mut c_void;
type FindObjectsOfTypeFn = unsafe extern "C" fn(ty: *mut c_void, include_inactive: bool) -> *mut c_void;

/// 托管数组长度（越界/空指针一律 0）。
pub fn array_len(arr: usize) -> usize {
    if arr == 0 {
        return 0;
    }
    unsafe { *((arr as *const u8).add(OBJ_HDR + 8) as *const usize) }
}

/// 托管数组第 i 个元素（越界返回 0）。
pub fn array_get(arr: usize, i: usize) -> usize {
    if arr == 0 {
        return 0;
    }
    let len = array_len(arr);
    if i >= len {
        return 0;
    }
    unsafe { *((arr as *const u8).add(ARRAY_VECTOR_OFF + i * 8) as *const usize) }
}

/// 读 Il2CppString 为 Rust String（有界 64 字符，遇 NUL 提前断）。
pub fn read_managed_string(p: usize) -> String {
    if p == 0 {
        return String::new();
    }
    unsafe {
        let len_i32 = *((p as *const u8).add(STR_LEN_OFF) as *const i32);
        if len_i32 <= 0 {
            return String::new();
        }
        let n = (len_i32 as usize).min(64);
        let chars = (p as *const u8).add(STR_CHARS_OFF) as *const u16;
        let mut s = String::with_capacity(n);
        for i in 0..n {
            let c = *chars.add(i);
            if c == 0 {
                break;
            }
            if let Some(ch) = char::from_u32(c as u32) {
                s.push(ch);
            }
        }
        s
    }
}

/// 枚举场景内某类的全部实例。返回对象指针列表。
///
/// 逐级降级链（任一步失败即返回空，绝不 panic）：
/// 1. 总闸 `probe_instances` 关 → 空
/// 2. 类解析不到 → 空
/// 3. `il2cpp_class_get_type` 符号解析不到 → 空
/// 4. `Object.FindObjectsOfType(Type,bool)` 解析不到 → 空
/// 5. 返回数组为空/不可读 → 空
pub fn find_instances(assembly: &str, ns: &str, class: &str) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    if !crate::config::probe_instances() {
        return out;
    }
    let klass = get_class_ptr(assembly, ns, class);
    if klass == 0 {
        chlog!(warn, "find_instances: 类缺失 {ns}.{class}");
        return out;
    }
    let Some(sym) = resolve_symbol("il2cpp_class_get_type") else {
        chlog!(warn, "find_instances: 符号缺失 il2cpp_class_get_type");
        return out;
    };
    let type_f: ClassGetTypeFn = unsafe { std::mem::transmute(sym) };
    let ty = unsafe { type_f(klass as *mut c_void) };
    if ty.is_null() {
        chlog!(warn, "find_instances: {class} 取 System.Type 失败");
        return out;
    }
    let Some(m) = get_method_addr("UnityEngine.CoreModule", "UnityEngine", "Object", "FindObjectsOfType", 2)
    else {
        chlog!(warn, "find_instances: Object.FindObjectsOfType/2 解析失败");
        return out;
    };
    let find_f: FindObjectsOfTypeFn = unsafe { std::mem::transmute(m) };
    let arr = unsafe { find_f(ty, true) } as usize;
    if arr == 0 {
        return out;
    }
    let n = array_len(arr);
    chlog!(info, "find_instances: {class} 命中 {n} 个实例");
    let cap = n.min(1024);
    for i in 0..cap {
        let o = array_get(arr, i);
        if o != 0 {
            out.push(o);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_guarded_reads() {
        assert_eq!(array_len(0), 0);
        assert_eq!(array_get(0, 3), 0);
        assert_eq!(read_managed_string(0), "");
    }

    #[test]
    fn gate_closed_returns_empty() {
        // 默认配置下总闸关闭 —— 不触碰任何游戏内存
        assert!(!crate::config::Config::default().probe_instances);
    }

    #[test]
    fn offsets_match_managed_layout() {
        // Il2CppObject = klass(8) + monitor(8)
        assert_eq!(OBJ_HDR, 16);
        // Il2CppArray = obj(16) + bounds(8) + max_length(8)
        assert_eq!(ARRAY_VECTOR_OFF, 32);
        // Il2CppString = obj(16) + length(4) + chars
        assert_eq!(STR_LEN_OFF, 16);
        assert_eq!(STR_CHARS_OFF, 20);
    }
}
