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

use host::{GetApiFn, cstr};
use std::ffi::c_char;

pub const PLUGIN_NAME: &str = "so-chonggou";
pub const PLUGIN_API_VERSION: i32 = 3;

/// 宿主插件入口（V3）。签名与 Hachimi-Edge plugin_api.rs 对齐。
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
        return 0; // InitResult::Error
    }

    log::info!("{PLUGIN_NAME}: attached in plugin mode (host api v{version})");
    host::bind(get_api);

    // 注册「游戏初始化完成」回调，再装 hook（此时 il2cbp 元数据已就绪）
    if let Some(f) = host::api_pub("hachimi_register_on_game_initialized") {
        type RegFn = unsafe extern "C" fn(cb: Option<unsafe extern "C" fn(*mut c_void)>, ud: *mut c_void) -> bool;
        let reg: RegFn = unsafe { std::mem::transmute(f) };
        unsafe { reg(Some(on_game_initialized), std::ptr::null_mut()) };
    } else {
        // 宿主不支持该回调时直接装
        hooks::install_all();
    }

    1 // InitResult::Ok
}

unsafe extern "C" fn on_game_initialized(_ud: *mut c_void) {
    log::info!("{PLUGIN_NAME}: game initialized, installing guarded hooks");
    hooks::install_all();
    host::host_log(3, "chonggou", "loaded (plugin mode, conflict guard active)");
}

/// 兼容 V2 宿主的旧入口（可选）。
#[no_mangle]
pub extern "C" fn hachimi_init(_vtable: *const c_char, _version: i32) -> i32 {
    // V2 vtable 布局与 V3 get_api 不同；本库要求 V3。
    log::warn!("{PLUGIN_NAME}: V2 init rejected, requires hachimi_init_v3");
    0
}

// 让 host 模块可从这里取 API（薄封装，保持 host::api 私有）
pub(crate) fn api_pub(name: &str) -> Option<usize> {
    host::api_lookup(name)
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn so_chonggou_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

// 防止未使用告警（cstr 工具在 hooks 扩展时使用）
#[allow(dead_code)]
fn _keep(s: *const c_char) -> &'static str {
    cstr(s)
}
