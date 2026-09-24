//! 宿主 Hachimi-Edge 的 hook 类**全量清单**（机器可读版，人类可读版见 docs/CONFLICTS.md）。
//!
//! 来源：逐个列目录 `Hachimi-Edge/src/il2cpp/hook/**` + `src/android/hook.rs` 实查。
//! 规则：
//! - **第 0 道闸**：清单内的类一律不 hook（hooks::install_all 里强制检查）；
//! - 清单外的目标还要过 guard 的 prologue 探测（第 2 道闸，按方法级地址判）；
//! - 进程级 hook 点（linker / JNI 函数表）由 guard::HOST_DENY_SYMBOLS 单独管。

/// 宿主已 hook 的 IL2CPP 类名（按程序集分组注释）。
pub const HOST_HOOKED_CLASSES: &[&str] = &[
    // ── CriMw.CriWare.Runtime（音频）──
    "CriAtomExAcb",
    "CriAtomExPlayback",
    "CriAtomExPlayer",
    "CriAtomSourceBase",
    // ── Cute.Core ──
    "Device",
    "SafetyNet",
    // ── Cute.Cri ──
    "AtomSourceEx",
    "AudioControllerBase",
    "AudioManager",
    "AudioPlayback",
    "CuteAudioSource",
    "CuteAudioSourcePool",
    "MovieManager",
    // ── Cute.UI ──
    "AtlasReference",
    // ── DOTween ──
    "TweenManager",
    // ── LibNative.Runtime（SQLite）──
    "Sqlite3Connection",
    "Sqlite3PreparedQuery",
    "Sqlite3Query",
    // ── Plugins/AnimateToUnity（Live 舞台动画）──
    "AnGlobalData",
    "AnKeyParameter",
    "AnMeshInfoParameterGroup",
    "AnMeshParameter",
    "AnMotionParameter",
    "AnMotionParameterGroup",
    "AnObjectParameterBase",
    "AnRoot",
    "AnRootParameter",
    "AnText",
    "AnTextParameter",
    // ── UnityEngine.AssetBundleModule ──
    "AssetBundle",
    "AssetBundleRequest",
    // ── UnityEngine.CoreModule ──
    "Application",
    "AsyncOperation",
    "Behaviour",
    "Camera",
    "Component",
    "GameObject",
    "Graphics",
    "Material",
    "Object",
    "QualitySettings",
    "RectOffset",
    "RectTransform",
    "RenderTexture",
    "Resources",
    "Scene",
    "SceneManager",
    "Screen",
    "Sprite",
    "Texture",
    "Texture2D",
    "TouchScreenKeyboard",
    "TouchScreenKeyboardType",
    "Transform",
    "UnityAction",
    // ── UnityEngine.ImageConversionModule ──
    "ImageConversion",
    // ── UnityEngine.InputLegacyModule ──
    "Input",
    // ── UnityEngine.TextRenderingModule ──
    "Font",
    "TextGenerator",
    "TextMesh",
    // ── UnityEngine.UI ──
    "CanvasScaler",
    "ContentSizeFitter",
    "EventSystem",
    "HorizontalOrVerticalLayoutGroup",
    "Image",
    "LayoutElement",
    "LayoutGroup",
    "LayoutRebuilder",
    "Text",
    "VerticalLayoutGroup",
    // ── UnityEngine.UIModule ──
    "Canvas",
    "CanvasGroup",
    // ── Unity.InputSystem ──
    "AxisControl",
    "ButtonControl",
    "DpadControl",
    "Gamepad",
    "InputControl",
    "Vector2Control",
    // ── UnityEngine.Rendering.Universal ──
    "ScriptableRenderer",
];

/// 类级 denylist 查询（精确匹配类名）。
pub fn class_denied(class: &str) -> bool {
    HOST_HOOKED_CLASSES.iter().any(|c| *c == class)
}

/// 我们自己的 hook 目标登记（供 /hooks 与审计；只是文档性登记）。
pub const OUR_HOOKED_CLASSES: &[&str] = &["PlayableDirector"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_classes_denied() {
        // 宿主清单里的几个代表
        assert!(class_denied("RectTransform"));
        assert!(class_denied("Application"));
        assert!(class_denied("TweenManager"));
        assert!(class_denied("AssetBundle"));
    }

    #[test]
    fn our_targets_not_in_host_list() {
        // 铁律：我们 hook 的类绝不能出现在宿主清单里
        for c in OUR_HOOKED_CLASSES {
            assert!(!class_denied(c), "{c} 与宿主冲突！");
        }
    }

    #[test]
    fn inventory_size_sane() {
        assert!(HOST_HOOKED_CLASSES.len() >= 70);
    }
}
