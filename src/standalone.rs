//! 独立模式：无宿主 Hachimi 时直接作为 libmain.so 注入。
//!
//! v1 设计（**零 hook 引擎**）：
//! - 不再自带 Dobby —— 全进程保持"只有宿主一套 inline patch"的原则；
//! - libil2cpp 的就绪检测不用 hook do_dlopen，改为后台线程 500ms 轮询
//!   `/proc/self/maps` —— 零侵入，等价信息；
//! - v1 限制（日志里会写明）：独立模式没有宿主 API，hooks::install_all 的
//!   il2cpp 解析会全部 MISS —— 本模式只提供 诊断HTTP + 配置/日志。
//!   完整 hook 在插件模式下使用。

use std::ffi::c_void;
use once_cell::sync::OnceCell;

#[cfg(target_os = "android")]
#[allow(dead_code)]
static JAVA_VM: OnceCell<jni::JavaVM> = OnceCell::new();

/// 与宿主相同的入口约定：接管 JNI_OnLoad 并链回原 libmain_orig.so。
#[cfg(target_os = "android")]
#[allow(non_snake_case, non_camel_case_types)]
#[no_mangle]
pub extern "C" fn JNI_OnLoad(vm: jni::JavaVM, reserved: *mut c_void) -> jni::sys::jint {
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("chonggou")
            .with_max_level(log::LevelFilter::Info),
    );
    chlog!(info, "standalone JNI_OnLoad（零 hook 引擎，maps 轮询模式）");

    crate::config::init(None);
    crate::http_diag::start(None);

    unsafe {
        let handle = libc::dlopen(
            std::ffi::CStr::from_bytes_with_nul(b"libmain_orig.so\0").unwrap().as_ptr(),
            libc::RTLD_LAZY,
        );
        if !handle.is_null() {
            type JniOnLoadFn = extern "C" fn(vm: jni::JavaVM, reserved: *mut c_void) -> jni::sys::jint;
            let orig_fn: JniOnLoadFn = std::mem::transmute(libc::dlsym(handle, c"JNI_OnLoad".as_ptr()));
            let vm_ptr = vm.get_java_vm_pointer();
            let _ = JAVA_VM.set(vm);
            start_il2cpp_watch();
            // JavaVM 已被 move 进 OnceCell，从 raw 重建一份给原函数
            return match jni::JavaVM::from_raw(vm_ptr) {
                Ok(vm_for_orig) => orig_fn(vm_for_orig, reserved),
                Err(e) => {
                    chlog!(error, "standalone: JavaVM::from_raw 失败({e:?})，跳过原 JNI_OnLoad");
                    -1
                }
            };
        }
    }
    chlog!(error, "libmain_orig.so not found — 该 SO 需配合重打包（原库改名 libmain_orig.so）");
    -1
}

/// 后台轮询 /proc/self/maps，等 libil2cpp.so 映射后触发解析。
/// 无 hook、无补丁 —— 纯观察。
#[cfg(target_os = "android")]
fn start_il2cpp_watch() {
    let spawned = std::thread::Builder::new()
        .name("chonggou-il2cpp-watch".to_owned())
        .spawn(|| loop {
            if maps_contain("libil2cpp.so") {
                chlog!(info, "standalone: libil2cpp 已映射（无宿主 API，install_all 的解析会全 MISS 并写日志）");
                crate::hooks::install_all();
                crate::training_anim::resolve_candidates();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        });
    if spawned.is_err() {
        chlog!(warn, "standalone: il2cpp 观察线程启动失败");
    }
}

/// /proc/self/maps 是否包含目标串（读失败按 false，不 panic）。
#[cfg(target_os = "android")]
fn maps_contain(needle: &str) -> bool {
    std::fs::read_to_string("/proc/self/maps")
        .map(|m| m.contains(needle))
        .unwrap_or(false)
}
