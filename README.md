# so-chonggou

防 hook 冲突的 IL2CPP 增强 SO —— Hachimi-Edge 的“重构筑版”，专为**无 root 开发者模式**设计：与宿主共存、互不覆盖对方的 hook 点。

## 为什么原版会冲突

Hachimi-Edge 在 Android 上占用以下 hook 点（见其 `src/android/hook.rs`）：

| hook 点 | 说明 |
|---|---|
| `linker64::__dl__Z9do_dlopenPKciPK17android_dlextinfoPKv` | 监听 so 加载（一次性，找到目标后自解）|
| `libc.so::dlopen` | 回退路径 / Waydroid 强制 |
| `JNINativeInterface::RegisterNatives` | 抓 `nativeInjectEvent` 地址（一次性）|
| 数百个 il2cpp 方法（Dobby inline patch） | 见 `src/il2cpp/hook/**` |

两个 SO 同时对同一函数做 inline patch → 后者覆盖前者的跳板 → 链断裂 / 直接崩。

## 本 SO 的双层防护

1. **静态 denylist**（`src/guard.rs`）：宿主已占用的符号直接拒绝安装。
2. **动态 prologue 探测**：装 hook 前读目标函数首指令，若已是 `B imm` / `LDR X16; BR X16` 跳板 —— 说明**任何引擎**已经 hook 过它，本库让路，绝不二次打补丁。
   > 谁先到谁说了算；后到者按 `AlreadyHooked` 跳过并写日志。

## 两种运行模式

### 插件模式（推荐，与 Hachimi-Edge 共存）
产物名 `libhachimi_chonggou.so`，命中宿主自动扫描前缀 `libhachimi_`；或手动加入 Hachimi 配置 `android.load_libraries`。

- 导出 `hachimi_init_v3`，宿主回调时注入 `get_api`
- **所有 hook 走宿主的 Interceptor**——全进程只有一套 inline hook 引擎
- il2cpp 解析、日志、GUI 通知全部借道宿主 API

### 独立模式（无宿主）
重打包：原 `libmain.so` → `libmain_orig.so`，本 SO 顶替 `libmain.so`。
自带 Dobby，仅此模式启用；进 `JNI_OnLoad` 链回原函数，等 `libil2cpp.so` 加载后再开 hook 窗口。

## 冲突矩阵（新增 hook 前先查）

| 目标 | 宿主 | 本 SO | 结论 |
|---|---|---|---|
| `do_dlopen` / `dlopen` | ✅ | 仅独立模式 | 共存时跳过 |
| `RegisterNatives` | ✅ | ❌ 不使用 | — |
| `Application::set_targetFrameRate` | ❌ | ✅ 示例 | 安全 |
| `Scene`/`AssetBundle`/`CriWare` 等宿主 hook 目录 | ✅ | 默认拒绝 | guard 自动跳过 |

新增 hook 规则：**只加宿主 `src/il2cpp/hook/**` 补集里的目标**，一律经 `host::hook_guarded()` 安装。

## 构建

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk
cargo ndk -t arm64-v8a build --release
# 产物: target/aarch64-linux-android/release/libhachimi_chonggou.so
```

CI：`.github/workflows/build.yml`（NDK + cargo-ndk，push 自动出 artifact）。

## 测试

```bash
cargo test          # guard 的 prologue 探测 / denylist 单测
adb logcat -s chonggou   # 装机后观察：hooked / skipped (conflict guard)
```

## License

GPLv3（与宿主一致）。
