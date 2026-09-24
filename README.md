# so-chonggou

防 hook 冲突的 IL2CPP 增强 SO —— Hachimi-Edge 的"重构筑版"，专为**无 root 开发者模式**设计：与宿主共存、互不覆盖对方的 hook 点、**每个可观察点都有日志**、自带诊断 HTTP 端点（对接 Agora）。

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

## 两种运行模式

### 插件模式（推荐，与 Hachimi-Edge 共存）
产物名 `libhachimi_chonggou.so`，命中宿主自动扫描前缀 `libhachimi_`；或手动加入 Hachimi 配置 `android.load_libraries`。

- 导出 `hachimi_init_v3`，宿主回调时注入 `get_api`
- **所有 hook 走宿主的 Interceptor** —— 全进程只有一套 inline hook 引擎
- il2cpp 解析、日志、GUI 通知全部借道宿主 API

### 独立模式（无宿主）
重打包：原 `libmain.so` → `libmain_orig.so`，本 SO 顶替 `libmain.so`。
自带 Dobby，仅此模式启用；进 `JNI_OnLoad` 链回原函数。v1 限制见 `src/standalone.rs` 头注释。

## 诊断 HTTP（Agora 对接口）

启动时按 `http_port`（默认 **18765**）bind `127.0.0.1`，**被占自动顺延**（18766→…）——
因为 **hlpatch SO 已经占用 18765**，两个 SO 可共存。实际端口写入：

```
<数据目录>/chonggou_port.txt     # 同时有日志记录
```

| 端点 | 内容 |
|---|---|
| `/health` | 存活 / 版本 / 实际端口 |
| `/status` | 配置 + 日志条数 + hook 数 |
| `/logs?n=200` | 最近 n 条全量日志（环形缓冲 1500 条）|
| `/hooks` | 我方 hook 注册表（orig/trampoline 地址）|
| `/anim` | 育成动画取证表（3帧化进度）|
| `/config` | 当前配置 + 配置文件路径 |

## 配置（`<数据目录>/chonggou.json`，首次运行自动生成）

```json
{
  "http_enabled": true,
  "http_port": 18765,
  "anim_mode": "measure",          // off | measure(v1) | speed(v2)
  "anim_speed_multiplier": 20.0,
  "target_frames": 3,
  "log_ui_positions": false
}
```

## 育成动画 3 帧化（来自 Rue 的 Windows 实测结论）

- **红线**：不能 skip（程序跳过事件 → 服务器收不到消息 → 卡死，必须退大厅救）；总 1 帧也不行
- **正确语义**：start 1帧 + loop 1帧 + end 1帧 = **3 帧最稳**（每阶段照常完整播、事件全部照发）
- **v1 = measure**：hook `PlayableDirector::Play`（宿主未占用）记录每次演出 + 对候选类
  （`SingleModeTrainingCutInHelper` 等 × 命名空间）做**只查址**探测，命中/落空全量进 `/anim`
- **v2 = speed**：拿 v1 装机一局的日志定谳真实命名空间后，把提速精确落到
  `Animator::set_speed` / director graph（配置 `target_frames`）

## 全域日志

每个观察点都落日志（宏 `chlog!`）：初始化、配置加载、HTTP 端口协商、每次 hook 安装/跳过、
每次 PlayableDirector 播放、探测命中/落空。三个去处：内存环形缓冲（1500 条，`/logs` 可拉）→
logcat（tag `chonggou`）→（v2）文件。

**"文字位置不对"类问题的排查**：配置 `log_ui_positions: true` 开启 UI 位置日志（v2 提供
`/uitree` 端点 dump RectTransform 位置）——不再靠截图肉眼比对。

## 冲突矩阵（新增 hook 前先查）

| 目标 | 宿主 | 本 SO | 结论 |
|---|---|---|---|
| `do_dlopen` / `dlopen` | ✅ | 仅独立模式 | 共存时跳过 |
| `RegisterNatives` | ✅ | ❌ 不使用 | — |
| `PlayableDirector::Play` | ❌ | ✅ 观测 | 安全 |
| `Application::set_targetFrameRate` | ❌ | ✅ 示例 | 安全 |
| `Scene`/`AssetBundle`/`CriWare`/`Text` 等宿主 hook 目录 | ✅ | 默认拒绝 | guard 自动跳过 |

新增 hook 规则：**只加宿主 `src/il2cpp/hook/**` 补集里的目标**，一律经 `host::hook_guarded()` 安装。

## 构建

```bash
rustup target add aarch64-linux-android
cargo install cargo-ndk
cargo ndk -t arm64-v8a build --release
# 产物: target/aarch64-linux-android/release/libhachimi_chonggou.so
```

CI：`.github/workflows/build.yml`（test + cargo-ndk，push 自动出 artifact）。

## 测试

```bash
cargo test              # guard 单测（prologue 探测/denylist/幂等）+ logging 环形缓冲单测
adb logcat -s chonggou  # 装机后观察
curl 127.0.0.1:18765/logs?n=500   # 或 Agora 直接读
```

## License

GPLv3（与宿主一致）。
