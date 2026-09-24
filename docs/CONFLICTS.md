# Hook 冲突全量清单（实查 Hachimi-Edge 源码树）

> 来源：逐个列目录 `Hachimi-Edge/src/il2cpp/hook/**` + `src/android/hook.rs`（2026-09 审查）。
> 本清单的机器可读版：`src/denylist.rs::HOST_HOOKED_CLASSES`，CI 有测试保证
> "我们 hook 的类 ∉ 宿主清单"（`our_targets_not_in_host_list`）。

## 一、进程级 hook 点（最危险，先崩就是这里）

| 目标 | 宿主行为 | 我们 |
|---|---|---|
| `linker64::__dl__Z9do_dlopen…PKv`（A8+） | hook 后监听 so 加载，找到目标即**自解** | 插件模式：不碰；独立模式：guard 兜底 |
| `linker64::__dl__Z9do_dlopen…Pv`（A7.x） | 同上 | 同上 |
| `libc.so::dlopen` | 回退路径 / Waydroid 强制 | 插件模式：不碰 |
| `JNINativeInterface::RegisterNatives` | 抓 `nativeInjectEvent`，抓到即**自解** | ❌ 永不使用 |
| `libmain.so` 顶替 + `libmain_orig.so` 链 | 宿主自己的注入方式 | 我们的独立模式沿用同机制 |

## 二、IL2CPP 类级 hook（inline patch，常驻）

### CriMw.CriWare.Runtime（音频）
CriAtomExAcb · CriAtomExPlayback · CriAtomExPlayer · CriAtomSourceBase

### Cute.Core
Device · SafetyNet

### Cute.Cri
AtomSourceEx · AudioControllerBase · AudioManager · AudioPlayback · CuteAudioSource · CuteAudioSourcePool · MovieManager

### Cute.UI
AtlasReference

### DOTween
TweenManager

### LibNative.Runtime（SQLite 明文化）
Sqlite3Connection · Sqlite3PreparedQuery · Sqlite3Query

### Plugins/AnimateToUnity（Live 舞台动画）
AnGlobalData · AnKeyParameter · AnMeshInfoParameterGroup · AnMeshParameter · AnMotionParameter · AnMotionParameterGroup · AnObjectParameterBase · AnRoot · AnRootParameter · AnText · AnTextParameter

### UnityEngine.AssetBundleModule
AssetBundle · AssetBundleRequest

### UnityEngine.CoreModule（**25 个类**）
Application · AsyncOperation · Behaviour · Camera · Component · GameObject · Graphics · Material · Object · QualitySettings · RectOffset · **RectTransform** · RenderTexture · Resources · Scene · SceneManager · Screen · Sprite · Texture · Texture2D · TouchScreenKeyboard · TouchScreenKeyboardType · Transform · UnityAction

### UnityEngine.ImageConversionModule
ImageConversion

### UnityEngine.InputLegacyModule
Input

### UnityEngine.TextRenderingModule
Font · TextGenerator · TextMesh

### UnityEngine.UI
CanvasScaler · ContentSizeFitter · EventSystem · HorizontalOrVerticalLayoutGroup · Image · LayoutElement · LayoutGroup · LayoutRebuilder · Text · VerticalLayoutGroup

### UnityEngine.UIModule
Canvas · CanvasGroup

### Unity.InputSystem
AxisControl · ButtonControl · DpadControl · Gamepad · InputControl · Vector2Control

### UnityEngine.Rendering.Universal
ScriptableRenderer

## 三、so-chonggou 的占用（清单外，零交集）

| 目标 | 用途 | 状态 |
|---|---|---|
| `PlayableDirector::Play()`（0 参） | 育成演出取证（3帧化 v1） | ✅ 类不在宿主清单 |
| `PlayableDirector::Play(asset)`（1 参） | 同上 | ✅ |

> 曾经的示例目标 `Application::set_targetFrameRate` 已**主动撤下**——Application 类
> 在宿主清单里，虽然具体方法大概率没被宿主 patch（prologue 探测能防），但按
> "类级零交集"纪律执行：宿主碰过的类我们一个不碰。

## 四、执行顺序（install_all 实装）

```
① class_denied(cls)     —— denylist.rs 清单，命中即拒（日志可见）
② get_method_addr       —— 借宿主 API 查址（查不到即跳过）
③ guard::decide(addr)   —— 符号 denylist + prologue 探测（B/LDR-BR 跳板 = 让路）
④ 宿主 Interceptor 安装 —— 全进程只有这一套 inline patch 引擎
⑤ chlog 登记            —— 成功/跳过/失败 全量日志，/hooks 端点可查
```

## 五、`/uitree` 与文字位置问题（v2 预告）

「文字位置不对」类问题的排查需要读 RectTransform——宿主 hook 了 RectTransform 类，
但我们**只读不 hook**（遍历层级树取 anchoredPosition/sizeDelta），不做 inline patch，
不进 denylist 冲突面。v2 落地，开关 `log_ui_positions`。
