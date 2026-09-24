# so-chonggou

防 hook 冲突的 IL2CPP 增强 SO —— Hachimi-Edge 的"重构筑版"，专为**无 root 开发者模式**设计：与宿主共存、互不覆盖对方的 hook 点、**每个可观察点都有日志**、自带诊断 HTTP 端点（对接 Agora）。

## 装载路径（不用"自己注入"）

本 SO **不做任何主动注入**（无 root 也不允许 ptrace 注入）：

```
Hachimi Edge 打包 APK 时把 libhachimi_chonggou.so 放进 lib/arm64-v8a/
        ↓ 游戏启动
Hachimi（libmain.so）JNI_OnLoad → 插件扫描器
        ↓ 自动扫描前缀 libhachimi_*.so
dlopen(libhachimi_chonggou.so) → hachimi_init_v3(get_api) → 我们挂上
```

宿主怎么进游戏，我们就怎么进——零额外注入面。独立模式（自己顶替 libmain.so）同样
是打包期替换，不是运行时注入。

## 为什么原版会冲突（清单在 docs/CONFLICTS.md）

Hachimi-Edge 实查结果：**进程级 3 处 + IL2CPP 类级 80+ 个**（RectTransform、Application、
SceneManager、AssetBundle、CriWare、DOTween、Text……全在内）。两个 SO 对同一函数
做 inline patch → 后者覆盖前者的跳板 → 链断裂 / 直接崩。

本 SO 的三层闸（`src/hooks.rs::install_all`）：

1. **类级 denylist**（`src/denylist.rs`，80+ 类全量收录）—— 宿主碰过的类我们一个不碰，CI 有测试锁死；
2. **prologue 探测**（`src/guard.rs`）—— 目标方法首指令已是 `B`/`LDR X16;BR X16` 跳板就让路；
3. **借宿主 Interceptor 安装** —— 全进程只有一套 inline patch 引擎。

## 诊断 HTTP（Agora 对接口）

启动时按 `http_port`（默认 **18765**）bind `127.0.0.1`，**被占自动顺延**（18766→…）——
因为 **hlpatch SO 已占用 18765**，两个 SO 可共存。实际端口写入外部媒体目录 +
宿主数据目录（best-effort）+ 日志。

| 端点 | 内容 |
|---|---|
| `/health` | 存活 / 版本 / 实际端口 |
| `/status` | 配置 + 日志条数 + hook 数 |
| `/logs?n=200` | 最近 n 条全量日志（环形缓冲 1500 条）|
| `/hooks` | 我方 hook 注册表（orig/trampoline 地址）|
| `/anim` | 育成动画取证表（3帧化进度）|
| `/config` | 当前配置 + 两个配置路径 |

## 配置（三层来源，坏了也不崩）

1. `/sdcard/Android/media/<包名>/hachimi/chonggou.json`（外部优先——文件管理器直接改）；
2. 宿主数据目录的 `chonggou.json`（开「使用内部文件目录」后在 /data/data，文件管理器摸不到，但进程内可读）；
3. 内存默认值（文件系统全不可用也照常运行）。

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

## 防闪退纪律（针对「使用内部文件目录」/ Android 16+ 错误2 场景）

- **本 SO 不解析、不创建、不依赖任何固定目录**——目录全来自宿主 API 或包名推导，取不到就降级；
- 所有 fs 操作走 Result，失败只记日志（错误2 = ENOENT 也一样）；
- 所有锁 poison 恢复（`unwrap_or_else(|p| p.into_inner())`），**游戏进程里零 panic 路径**；
- 配置/端口文件双位置写入，单边失败无感。

## 育成动画 3 帧化（来自 Rue 的 Windows 实测结论）

- **红线**：不能 skip（程序跳过事件 → 服务器收不到消息 → 卡死，必须退大厅救）；总 1 帧也不行
- **正确语义**：start 1帧 + loop 1帧 + end 1帧 = **3 帧最稳**（每阶段照常完整播、事件全部照发）
- **v1 = measure**：hook `PlayableDirector::Play`（类不在宿主清单）记录每次演出 + 对候选类
  （`SingleModeTrainingCutInHelper` 等 × 6 命名空间）做**只查址**探测，命中/落空全量进 `/anim`
- **v2 = speed**：拿 v1 装机一局的日志定谳真实命名空间后，把提速精确落到
  `Animator::set_speed` / director graph（配置 `target_frames`）

## 全域日志

每个观察点都落日志（宏 `chlog!`）：初始化、配置来源、HTTP 端口协商、每次 hook 安装/拒绝/跳过、
每次 PlayableDirector 播放、探测命中/落空。去处：内存环形缓冲（1500 条，`/logs` 可拉）→
logcat（tag `chonggou`）→（v2）文件。

**"文字位置不对"类问题**：v2 提供 `/uitree` 端点**只读**遍历 RectTransform 层级树
（不做 inline patch，不碰宿主 hook 面），开关 `log_ui_positions`。

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
cargo test              # guard（探测/denylist/幂等）+ logging（环形缓冲）+ config（解析/降级）单测
adb logcat -s chonggou  # 装机后观察
curl 127.0.0.1:18765/logs?n=500   # 或 Agora 直接读
```

## License

GPLv3（与宿主一致）。
