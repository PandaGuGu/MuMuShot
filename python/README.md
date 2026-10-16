# MuMuShot · Python 版

截取电脑屏幕 → 通过 adb 一键送进 MuMu 模拟器相册。

这是项目的**参考实现**（Rust 版在 [`../rust/`](../rust/README.md)，逐行对照本目录移植）。

## 它解决什么问题

MuMu 模拟器（以及大多数安卓模拟器）都没有"从电脑取图"的能力。想给模拟器里的 App
（微信、抖音、小红书、闲鱼）发一张电脑上的截图，只能手动 `adb push` 进 `/sdcard`，
还得想办法让媒体库刷新，否则相册里根本不显示。MuMuShot 把这一串操作压缩成一个热键。

## 运行

需要 Python 3.10+（要带 tkinter）和 Pillow：

```bash
pip install pillow
python mumu_shot.py
```

也可以直接双击 `mumu-shot.bat`（用绑定的解释器从源码启动，不用打包）。

## 用法

不带参数启动会弹出小面板，**保持它开着**即可（关掉窗口 = 退出，热键同时失效）。

| 热键 | 作用 |
| --- | --- |
| `Ctrl + Alt + A` | 框选一块区域截图上传（自带变暗遮罩，最快） |
| `Ctrl + Alt + Q` | 调起系统截图工具框选，选完自动上传（可标注、可吸附窗口） |
| `Ctrl + Alt + S` | 全屏截图上传 |
| `Ctrl + Alt + C` | 上传剪贴板里当前的图片 |

命令行也全部支持（以下命令都在本目录下执行）：

```bash
python mumu_shot.py --full                      # 全屏截图上传
python mumu_shot.py --region                    # 交互框选
python mumu_shot.py --region 100,100,800,600    # 固定区域
python mumu_shot.py --snip                      # 调系统截图工具
python mumu_shot.py --clipboard                 # 上传剪贴板图片
python mumu_shot.py --window MuMu               # 截取指定标题的窗口
python mumu_shot.py --list-windows              # 列出所有窗口标题
python mumu_shot.py --console                   # 控制台模式（无图形面板）
python mumu_shot.py --selftest                  # 自检后退出
python mumu_shot.py --open-gallery              # 上传后顺手在模拟器里打开
python mumu_shot.py --device 127.0.0.1:16384    # 手动指定实例（多开时用）
```

### 退出码

| 场景 | 退出码 |
| --- | --- |
| 成功 | `0` |
| `--region` 格式错误 | `2` |
| 连不上 adb / 设备 | `3` |
| 截图或推送失败 | `4` |
| `--window` 没匹配到窗口 | `5` |

### 截图后的两个副本

- 模拟器侧：`/sdcard/Pictures/MuMuShot/{标签}_{时间戳}.png`，标签有
  `Shot_` / `Region_` / `Snip_` / `Clip_` / `Win_` 五种
- 本机侧：exe（或脚本）旁边的 `shots/` 里留一份同样的文件
- 日志：旁边的 `mumu-tool.log`，超过 256 KB 自动轮转为 `.log.old`

## 设备识别

不给 `--device` 时全自动：先问 MuMuManager 要实例的 adb 端口，取不到再扫客户机进程的监听端口，
最后才试常见端口。后两级属于"猜"，所以做了两层保护：

- 候选先按**进程身份**筛 —— 只收客户机进程（`MuMuNxDevice.exe`）持有的端口；其它 MuMu 进程
  持有的高位数端口是自家 RPC 端口，在调用 adb 之前就丢掉（否则每个要白等约 10 秒握手超时）。
- 猜来的端口用 **2 秒短握手超时**，判定时把「取 Android 版本号 + 探 `/sdcard/Pictures` 可用」
  合并进**同一次** `adb shell` —— 判定更严，但不增加任何一次往返。

平时识别只花约 **0.15 秒**；模拟器没启动时约 **0.53 秒**就报出准确原因（而不是先去把常见端口
猜一遍）。识别顺序与 Rust 版逐行一致，详见根目录 README 的「端口是怎么自动找到的」与
[`docs/性能评测报告.md`](../docs/性能评测报告.md) §7.4。

## 打包

```bash
pip install pyinstaller pillow
python tools/rebuild_exe.py
```

打好的 `MuMuShot.exe` 与它必需的 `_internal/` 会放回**项目根目录**
（也就是你平时双击运行的那一份）。脚本只替换这两项，同目录下的源码、`.git`、
`docs/`、`rust/` 一概不动。想改落地点就设环境变量 `MUMUSHOT_DEST`。

也可以双击 `tools/rebuild-exe.bat`。

打 tag（`v1.1.0` 这样）推上来，GitHub Actions 会在 Windows runner 上同时构建两版：
Python 版打成 `MuMuShot.zip`，Rust 版编译出 `MuMuShot-Rust.exe`，一起发到 Releases。
Python 版发布前会跑一遍自检（认日志里的 `selftest ok`），不过就不发布。

## 目录

```
python/
├── mumu_shot.py          截图工具主程序（热键、面板、四种截图方式）
├── mumu_core.py          MuMu 公共能力：adb 自动发现 / 推送 / 媒体库扫描
├── mumu-shot.bat         双击从源码启动
└── tools/
    ├── make_icons.py     生成图标
    ├── rebuild_exe.py    一键重新打包成 exe
    ├── rebuild-exe.bat   Windows 下双击重新打包
    └── shot.ico/.png     图标
```
