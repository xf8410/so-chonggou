//! 独立模式：无宿主 Hachimi 时直接作为 libmain.so 注入。
//! 仅在此模式下使用本 SO 自带的 Dobby；与宿主共存时该模式不会被触发。

use std::ffi::{c_char, c_void, CStr};
use jni::{JavaVM, sys::jint};
use once_cell::sync::OnceCell;

use crate::guard;

static JAVA_VM: OnceCell<JavaVM> = OnceCell::new();

pub fn java_vm() -> Option<&'static JavaVM> {
    JAVA_VM.get()
}

#[allow(non_camel_case_types)]
type JniOnLoadFn = extern "C" fn(vm: JavaVM, reserved: *mut c_void) -> jint;

const LIBRARY_NAME: &CStr = c"libmain_orig.so";

/// 与宿主相同的入口约定：接管 JNI_OnLoad 并链回原 libmain_orig.so。
#[allow(non_snake_case)]
#[no_mangle]
pub extern "C" fn JNI_OnLoad(vm: JavaVM, reserved: *mut c_void) -> jint {
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("chonggou")
            .with_max_level(log::LevelFilter::Info),
    );
    log::info!("chonggou standalone JNI_OnLoad");

    unsafe {
        let handle = libc::dlopen(LIBRARY_NAME.as_ptr(), libc::RTLD_LAZY);
        if !handle.is_null() {
            let orig: JniOnLoadFn = std::mem::transmute(libc::dlsym(handle, c"JNI_OnLoad".as_ptr()));
            let _ = JAVA_VM.set(vm);
            install_standalone_hooks();
            return orig(vm, reserved);
        }
    }
    log::error!("libmain_orig.so not found — 该 SO 需配合重打包（原库改名 libmain_orig.so）");
    -1
}

/// 独立模式 hook 安装：同样先过 guard（denylist + prologue 探测）。
fn install_standalone_hooks() {
    // 独立模式下我们独占进程，denylist 主要防重复注入；prologue 探测防二次 hook。
    // do_dlopen 观察点（与宿主相同的语义，但仅在宿主不存在时才会走到这里）
    let addr = unsafe {
        dobby_rs::resolve_symbol("linker64", "__dl__Z9do_dlopenPKciPK17android_dlextinfoPKv")
    };
    match addr {
        Some(a) if !a.is_null() => match guard::decide(a as usize, Some("__dl__Z9do_dlopenPKciPK17android_dlextinfoPKv")) {
            guard::HookDecision::Allow(target) => {
                match unsafe { dobby_rs::hook(target as *mut c_void, on_do_dlopen as *mut c_void) } {
                    Ok(tramp) => {
                        guard::register_own_hook(target as usize, on_do_dlopen as usize);
                        DO_DLOPEN_TRAMP.store(tramp as usize, std::sync::atomic::Ordering::Release);
                        log::info!("standalone: hooked do_dlopen @ {target:#x}");
                    }
                    Err(e) => log::warn!("standalone: hook failed: {e:?}"),
                }
            }
            other => log::info!("standalone: do_dlopen skipped ({other:?})"),
        },
        _ => log::warn!("standalone: do_dlopen symbol unresolved"),
    }
}

static DO_DLOPEN_TRAMP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

type DoDlopenFn = extern "C" fn(filename: *const c_char, flags: i32, extinfo: *const c_void, caller: *const c_void) -> *mut c_void;
extern "C" fn on_do_dlopen(filename: *const c_char, flags: i32, extinfo: *const c_void, caller: *const c_void) -> *mut c_void {
    let tramp: DoDlopenFn = unsafe {
        std::mem::transmute(DO_DLOPEN_TRAMP.load(std::sync::atomic::Ordering::Acquire))
    };
    let handle = tramp(filename, flags, extinfo, caller);
    if !filename.is_null() {
        let name = unsafe { CStr::from_ptr(filename) }.to_string_lossy();
        if name.contains("libil2cpp") {
            log::info!("standalone: libil2cpp loaded, hook window open");
            crate::hooks::install_all();
        }
    }
    handle
}
