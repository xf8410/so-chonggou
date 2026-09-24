//! so-chonggou：防 hook 冲突的 IL2CPP 增强 SO
//!
//! 双模式入口：
//! 1. **插件模式（推荐，与 Hachimi-Edge 共存）** —— 宿主扫描到
//!    `libhachimi_chonggou.so` 后调用 `hachimi_init_v3`，本库借用宿主
//!    Interceptor 安装 hook，不带第二套 hook 引擎。
//! 2. **独立模式** —— 作为 `libmain.so` 注入（原库改名 `libmain_orig.so`），
//!    自带 Dobby，仅在无宿主时触发。

pub mod guard;
pub mod host;
pub mod hooks;

#[cfg(target_os = "android")]
pub mod standalone;

use host::GetApiFn;
use std::os::raw::c_void;

pub const PLUGIN_NAME: &str = "so-chonggou";

/// 宿主插件入口（V3）。签名与 Hachimi-Edge plugin_api.rs 对齐：
/// `hachimi_init_v3(get_api, version) -> InitResult(0=Error, 1=Ok)`
#[no_mangle]
pub extern "C" fn hachimi_init_v3(get_api: GetApiFn, version: i32) -> i32 {
    #[cfg(target_os = "android")]
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("chonggou")
            .with_max_level(log::LevelFilter::Info),
    );

    if version < 2 {
        log::error!("{PLUGIN_NAME}: host api version {version} too old");
        return 0;
    }

    log::info!("{PLUGIN_NAME}: attached in plugin mode (host api v{version})");
    host::bind(get_api);

    // 注册「游戏初始化完成」回调，再装 hook（此时 il2cpp 元数据已就绪）
    if let Some(f) = host::api_lookup("hachimi_register_on_game_initialized") {
        type RegFn = unsafe extern "C" fn(cb: Option<unsafe extern "C" fn(*mut c_void)>, ud: *mut c_void) -> bool;
        let reg: RegFn = unsafe { std::mem::transmute(f) };
        if unsafe { reg(Some(on_game_initialized), std::ptr::null_mut()) } {
            return 1;
        }
    }
    // 宿主不支持该回调时直接装
    hooks::install_all();
    1
}

unsafe extern "C" fn on_game_initialized(_ud: *mut c_void) {
    log::info!("{PLUGIN_NAME}: game initialized, installing guarded hooks");
    hooks::install_all();
    host::host_log(3, "chonggou", "loaded (plugin mode, conflict guard active)");
}

/// 兼容 V2 宿主的旧入口 —— 本库要求 V3 get_api，V2 拒绝。
#[no_mangle]
pub extern "C" fn hachimi_init(_vtable: *const std::os::raw::c_char, _version: i32) -> i32 {
    log::warn!("{PLUGIN_NAME}: V2 init rejected, requires hachimi_init_v3");
    0
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn so_chonggou_version() -> *const std::os::raw::c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const std::os::raw::c_char
}
