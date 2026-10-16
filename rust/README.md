# MuMuShot · Rust 版

截取电脑屏幕 → 通过 adb 一键送进 MuMu 模拟器相册。

本目录是 [`python/mumu_shot.py`](../python/mumu_shot.py) / [`python/mumu_core.py`](../python/mumu_core.py) 的 **Rust 重写**，
逐行对照参考实现移植：功能、CLI 参数、退出码、日志文案、文件命名全部对齐。

## 它解决什么问题

MuMu 模拟器（以及大多数安卓模拟器）都没有"从电脑取图"的能力。想给模拟器里的 App
（微信、抖音、小红书、闲鱼）发一张电脑上的截图，只能手动 `adb push` 进 `/sdcard`，
还得想办法让媒体库刷新，否则相册里根本不显示。MuMuShot 把这一串操作压缩成一个热键。

## 编译

```bash
cd rust
cargo build --release
# 产物：rust/target/release/MuMuShot.exe（单文件 5.83 MB，无外部依赖）
```

需要 Rust 1.85+（用到 edition 2024）。**仅支持 Windows。**

## 用法（与 Python 版一致）

```
MuMuShot.exe                             双击 → 打开图形面板 + 注册全局热键
MuMuShot.exe --full                      全屏截图上传（默认动作）
MuMuShot.exe --region                    交互框选截图上传
MuMuShot.exe --region 100,100,800,600    固定区域截图上传
MuMuShot.exe --snip                      调起系统截图工具（Win+Shift+S）框选后上传
MuMuShot.exe --clipboard                 上传剪贴板里的图片
MuMuShot.exe --window 关键词              截取标题包含该关键词的窗口
MuMuShot.exe --list-windows              列出所有窗口标题
MuMuShot.exe --console                   控制台模式（无图形面板，热键照常用）
MuMuShot.exe --selftest                  自检后退出
```

可选参数：`--snip-timeout`（默认 90 秒）、`--open-gallery`、`--hide-panel-on-shot`、
`--remote-dir`（默认 `/sdcard/Pictures/MuMuShot`）、`--adb`、`--device`。

### 退出码（与 Python 版一致）

| 场景 | 退出码 |
| --- | --- |
| 成功 | `0` |
| `--region` 格式错误 | `2` |
| 连不上 adb / 设备 | `3` |
| 截图或推送失败 | `4` |
| `--window` 没匹配到窗口 | `5` |

### 全局热键

| 热键 | 动作 |
| --- | --- |
| `Ctrl + Alt + A` | 框选截图上传 |
| `Ctrl + Alt + Q` | 调起系统截图工具框选 |
| `Ctrl + Alt + S` | 全屏截图上传 |
| `Ctrl + Alt + C` | 上传剪贴板里的图片 |

## 功能对照

| 能力 | Python 版 | Rust 版 |
| --- | --- | --- |
| 全屏截图（多显示器虚拟桌面） | `PIL.ImageGrab(all_screens=True)` | GDI `BitBlt` + `GetDIBits` |
| 交互框选（整屏压暗 + 拖拽） | tkinter 全屏 `Toplevel` | egui 无边框置顶视口 |
| 系统截图工具框选 | `ms-screenclip:` 协议 / 模拟 Win+Shift+S | 同左 |
| 剪贴板取图 | `ImageGrab.grabclipboard()`，支持位图与文件列表 | arboard 位图 + `CF_HDROP` 文件列表回退 |
| 指定窗口截图 | `EnumWindows` + `DwmGetWindowAttribute`，过滤 `WS_EX_TOOLWINDOW` | 同左（截屏幕对应区域） |
| 图形面板 | tkinter，410×400，置顶 | egui，410×400，置顶 |
| 全局热键 | `RegisterHotKey` + 消息循环，`MOD_NOREPEAT` | `RegisterHotKey`（global-hotkey）+ 工作线程消息循环，按住去重等价 NOREPEAT |
| 设备发现 | 显式 → MuMuManager → 进程端口 → 常见端口 → `adb devices` | 同左，顺序与超时一致 |
| adb 推送 + 媒体库索引 | `content call scan_volume`；6×0.8s 轮询 media id | 同左 |
| 本机留档 | `{标签}_{时间戳}.png`（`Shot_`/`Region_`/`Snip_`/`Clip_`/`Win_`） | 同左 |
| 日志 | `mumu-tool.log`，超 256 KB 轮转 `.old`，同时写控制台 | 同左 |
| 无控制台时错误提示 | `MessageBoxW`（仅在没有 stdout 时） | 同左（用 `GetConsoleWindow` 判断） |

## 实现要点

- **截图**：不引第三方截图库，直接用 Win32 GDI（`BitBlt` + `GetDIBits`）。
  进程启动即 `SetProcessDpiAwareness(PER_MONITOR_DPI_AWARE)`，保证坐标与物理像素一一对应。
- **框选覆盖层**：先抓一张整块虚拟桌面的图，再用一个无边框、置顶、铺满虚拟桌面的
  egui 视口把它当底图显示；拖拽矩形按比例映射回图像像素后裁剪 —— 天然支持多显示器。
  起点用 egui 的 `press_origin()` 取真正的按下位置，避免拖拽阈值让选区偏小。
- **热键线程**：`RegisterHotKey` 要求创建线程带消息循环，所以单独起一个线程常驻
  `PeekMessage` 泵，触发的事件通过全局 channel 交给界面线程。
- **面板生命周期**：eframe 0.37 的 `App` 拆成 `logic`（每遍都跑，窗口藏起来时也跑）
  与 `ui`（只在窗口可见时跑）。"截图瞬间隐藏面板"正是靠 `logic` 在隐藏期间仍能收到
  后台抓好的图，再恢复可见性并打开覆盖层。
- **无控制台也不崩**：窗口版双击启动时 stdout 句柄是空的，Rust 的 `println!` 会 panic，
  所以日志改用不会 panic 的写法。
- **中文界面**：egui 内置字体没有汉字，启动时从 `C:\Windows\Fonts` 加载
  `msyh.ttc`（微软雅黑）作为首选字体。
- **adb 参数**：`adb push` 的目标始终传**完整远端路径**；同时把工作目录切到截图目录、
  只传相对文件名，彻底避开非 ASCII 路径下 adb 推导文件名出错的问题。
- **设备识别不花时间也不猜**：识别顺序与 Python 版逐行对齐（MuMuManager → 客户端进程
  监听端口 → 常见端口 → `adb devices`）。后两级属于"猜"，所以候选先按进程身份筛（只收
  客户机进程持有的端口），再用 2 秒短握手超时兜住最坏耗时，判定时把「取 Android 版本号
  + 探 `/sdcard/Pictures`」合并进同一次 `adb shell` —— 判定更严但不增加任何往返。

## 性能

同一台机器、同一 MuMu 实例的对比（详见 [`docs/性能评测报告.md`](../docs/性能评测报告.md)）：

| 指标 | Python 版 | Rust 版 |
| --- | ---: | ---: |
| 发行体积 | 39.29 MB / 996 文件 | **5.83 MB / 1 文件** |
| `--selftest` 纯启动 | 234 ms | **123 ms** |
| 面板可见 | 762 ms | **45 ms** |
| `--full` 端到端 | 2867 ms | **2738 ms** |
| 自动识别设备开销 | 149 ms | 170 ms |
| 面板常驻内存 | **37.3 MB** | 98.5 MB |

端到端只快 5%，因为耗时主要花在 adb 进程往返与模拟器媒体库索引上，与语言无关。
内存方面 Rust 反而更高：egui/OpenGL 渲染栈加上进程内加载的完整中文字体（约 19 MB）。

## 测试

```bash
cargo test --release                            # 候选筛选等纯逻辑，随时可跑
cargo test --release -- --ignored --nocapture   # 真机：逐个候选端口的判定与耗时（需 adb + 运行中的模拟器）
```

## 目录

```
rust/
├── Cargo.toml
├── README.md
└── src/
    ├── main.rs      CLI 解析 + 模式分派 + 自检（对应 python/mumu_shot.py 的 main/run_cli/selftest）
    ├── app.rs       截图→上传统一入口（对应 Uploader）
    ├── adb.rs       设备发现 / adb 推送 / 媒体库索引（对应 mumu_core.py）
    ├── capture.rs   GDI 截图 / 剪贴板 / 窗口枚举 / 系统截图工具
    ├── gui.rs       图形面板 + 框选覆盖层 + 窗口选择（对应 run_gui / select_region）
    ├── hotkeys.rs   全局热键（对应 Hotkeys）
    └── core.rs      路径 / 日志 / 错误弹框
```
