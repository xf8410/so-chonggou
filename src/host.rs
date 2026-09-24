//! 宿主 API 层：插件模式下所有 il2cpp 访问与 hook 都借道宿主，
//! 本 SO **不引入第二套 inline hook 引擎**，从根源上避免冲突。

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
