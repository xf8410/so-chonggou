//! `/uitree` —— 「文字位置不对」排查端点。
//!
//! 做法：**平铺**列出场景里全部 `RectTransform`（名字 + anchoredPosition +
//! sizeDelta + localScale + 有无父节点），封顶 `uitree_max` 条。谁被谁顶歪了，
//! 一眼能看出来 —— 不用截图肉眼看。
//!
//! 为什么不递归走树：递归要 `Transform.GetChild` 逐层展开，一个 UI 动辄上千节点，
//! 在游戏渲染线程之外频繁调用风险与成本都不划算。平铺 + `has_parent` 已经够定位。
//!
//! ⚠️ 纪律：
//! - 整条链受 `config::probe_instances` 总闸保护（**默认关**）。关着时本端点
//!   照常返回 200，只带 `enabled:false` 与降级原因，不崩、不猜。
//! - 任一方法解析不到就**逐字段降级为 null**，绝不拿猜来的 ABI 去调。
//! - `RectTransform` / `Transform` / `Object` 虽在宿主 hook 清单里（docs/CONFLICTS.md），
//!   但 denylist 只管**装 hook**；我们这里只**读**，不装任何 hook，无冲突面。

use std::os::raw::c_void;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde_json::json;

use crate::host;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

type GetNameFn = unsafe extern "C" fn(this: *mut c_void) -> *mut c_void;
type GetVec2Fn = unsafe extern "C" fn(this: *mut c_void) -> Vec2;
type GetVec3Fn = unsafe extern "C" fn(this: *mut c_void) -> Vec3;
type GetParentFn = unsafe extern "C" fn(this: *mut c_void) -> *mut c_void;

#[derive(Clone, Copy, Default)]
struct Apis {
    get_name: Option<usize>,
    get_anchored: Option<usize>,
    get_size_delta: Option<usize>,
    get_local_scale: Option<usize>,
    get_parent: Option<usize>,
}

const ASM: &str = "UnityEngine.CoreModule";
const NS: &str = "UnityEngine";

fn resolve() -> Apis {
    let g = |cls: &str, m: &str, a: i32| host::get_method_addr(ASM, NS, cls, m, a);
    Apis {
        get_name: g("Object", "get_name", 0),
        get_anchored: g("RectTransform", "get_anchoredPosition", 0),
        get_size_delta: g("RectTransform", "get_sizeDelta", 0),
        get_local_scale: g("Transform", "get_localScale", 0),
        get_parent: g("Transform", "get_parent", 0),
    }
}

fn apis_report(a: &Apis) -> serde_json::Value {
    json!({
        "get_name":             a.get_name.is_some(),
        "get_anchoredPosition": a.get_anchored.is_some(),
        "get_sizeDelta":        a.get_size_delta.is_some(),
        "get_localScale":       a.get_local_scale.is_some(),
        "get_parent":           a.get_parent.is_some(),
    })
}

fn call2(f: usize, obj: usize) -> Option<Vec2> {
    if f == 0 {
        return None;
    }
    let g: GetVec2Fn = unsafe { std::mem::transmute(f) };
    Some(unsafe { g(obj as *mut c_void) })
}

fn call3(f: usize, obj: usize) -> Option<Vec3> {
    if f == 0 {
        return None;
    }
    let g: GetVec3Fn = unsafe { std::mem::transmute(f) };
    Some(unsafe { g(obj as *mut c_void) })
}

fn call_ptr(f: usize, obj: usize) -> Option<usize> {
    if f == 0 {
        return None;
    }
    let g: GetParentFn = unsafe { std::mem::transmute(f) };
    Some(unsafe { g(obj as *mut c_void) as usize })
}

fn name_of(f: usize, obj: usize) -> String {
    if f == 0 {
        return String::from("<no get_name>");
    }
    let g: GetNameFn = unsafe { std::mem::transmute(f) };
    let s = unsafe { g(obj as *mut c_void) } as usize;
    let out = host::read_managed_string(s);
    if out.is_empty() {
        String::from("<>")
    } else {
        out
    }
}

fn v2_json(v: Option<Vec2>) -> serde_json::Value {
    match v {
        Some(v) => json!({"x": v.x, "y": v.y}),
        None => serde_json::Value::Null,
    }
}

fn v3_json(v: Option<Vec3>) -> serde_json::Value {
    match v {
        Some(v) => json!({"x": v.x, "y": v.y, "z": v.z}),
        None => serde_json::Value::Null,
    }
}

static LAST_DUMP: Lazy<Mutex<serde_json::Value>> =
    Lazy::new(|| Mutex::new(json!({"enabled": false, "reason": "尚未 dump"})));

/// 抓一次全量 RectTransform 快照（结果缓存，`/uitree` 读缓存）。
pub fn refresh() {
    let cfg = crate::config::get();

    if !cfg.probe_instances {
        *LAST_DUMP.lock().unwrap_or_else(|p| p.into_inner()) = json!({
            "enabled": false,
            "reason": "probe_instances=false（默认关）。改 chonggou.json: {\"probe_instances\":true} 后重启游戏",
            "resolved": {},
            "instances": 0,
            "nodes": [],
        });
        return;
    }

    let a = resolve();
    let objs = host::find_instances(ASM, NS, "RectTransform");
    let cap = cfg.uitree_max as usize;
    let mut nodes: Vec<serde_json::Value> = Vec::with_capacity(objs.len().min(cap));

    for obj in objs.iter().take(cap) {
        nodes.push(json!({
            "obj": format!("{obj:#x}"),
            "name": name_of(a.get_name.unwrap_or(0), *obj),
            "anchoredPosition": v2_json(call2(a.get_anchored.unwrap_or(0), *obj)),
            "sizeDelta":        v2_json(call2(a.get_size_delta.unwrap_or(0), *obj)),
            "localScale":       v3_json(call3(a.get_local_scale.unwrap_or(0), *obj)),
            "has_parent":       call_ptr(a.get_parent.unwrap_or(0), *obj).map(|p| p != 0),
        }));
    }

    let out = json!({
        "enabled": true,
        "instances": objs.len(),
        "reported": nodes.len(),
        "cap": cap,
        "truncated": objs.len() > cap,
        "resolved": apis_report(&a),
        "nodes": nodes,
    });

    chlog!(info, "/uitree 刷新：命中 {} 个 RectTransform，吐出 {}", objs.len(), nodes.len());
    *LAST_DUMP.lock().unwrap_or_else(|p| p.into_inner()) = out;
}

/// `/uitree` 端点体。
pub fn snapshot_json() -> String {
    let v = LAST_DUMP.lock().unwrap_or_else(|p| p.into_inner());
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_api_degrades_to_null_not_crash() {
        // 全 0 地址 = 全部符号缺失，必须降级成 null 而不是崩
        assert!(v2_json(call2(0, 0xDEAD)).is_null());
        assert!(v3_json(call3(0, 0xDEAD)).is_null());
        assert!(call_ptr(0, 0xDEAD).is_none());
        assert_eq!(name_of(0, 0xDEAD), "<no get_name>");
    }

    #[test]
    fn vec_structs_are_c_compatible() {
        use std::mem::size_of;
        assert_eq!(size_of::<Vec2>(), 8);
        assert_eq!(size_of::<Vec3>(), 12);
    }

    #[test]
    fn probe_gate_defaults_off() {
        // 纯读断言：不动全局配置，测试并行也安全
        assert!(!crate::config::Config::default().probe_instances);
    }

    #[test]
    fn snapshot_always_returns_json() {
        // 未 refresh 过也必须是合法 JSON（端点不能吐半截）
        let s = snapshot_json();
        assert!(s.starts_with('{'));
    }
}
