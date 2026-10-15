<div align="center">
  <img src="tools/shot.png" width="110" alt="MuMuShot">
  <h1>MuMuShot</h1>
  <p>把电脑屏幕截下来，一键送进 MuMu 模拟器的相册</p>
</div>

---

MuMu 模拟器（以及大多数安卓模拟器）都没有"从电脑取图"的能力。想给模拟器里的 App —— 微信、抖音、小红书、闲鱼 —— 发一张电脑上的截图，只能手动 `adb push` 进 `/sdcard`，还得想办法让媒体库刷新，否则相册里根本不显示。

MuMuShot 把这一串操作压缩成一个热键。

```mermaid
flowchart LR
    A[电脑屏幕] -->|截图| B[本机 png]
    B -->|adb push| C[/sdcard/Pictures/MuMuShot/]
    C -->|触发媒体库扫描| D[模拟器相册 立刻可见]
```

## 特性

- **4 种截图方式** —— 自绘遮罩框选、调系统截图工具、全屏、直接上传剪贴板
- **4 个全局热键** —— 不用切回面板，在任何窗口按下即截图上传
- **自动找到模拟器** —— 不写死 adb 端口，自动识别实例并连接；模拟器正在开机时会自动等待
- **自动进相册** —— 推送后触发 MediaStore 扫描，图库/微信/抖音立刻能选到这张图
- **本机留档** —— 每张截图在本机也存一份，方便回溯

## 快速开始

### 方式一：直接下 exe（推荐）

到 [Releases](https://github.com/PandaGuGu/MuMuShot/releases) 下载 `MuMuShot.zip`，解压后双击 `MuMuShot.exe`。

> 整个文件夹要一起保留 —— 它旁边的 `_internal` 是必需的，不能只单独拷 exe。

### 方式二：从源码运行

需要 Python 3.10+（要带 tkinter）和 Pillow：

```bash
pip install pillow
python mumu_shot.py
```

## 用法

双击后会弹出一个小面板，**保持它开着**即可（关掉窗口 = 退出，热键同时失效）。

| 热键 | 作用 |
| --- | --- |
| `Ctrl + Alt + A` | 框选一块区域截图上传（自带变暗遮罩，最快） |
| `Ctrl + Alt + Q` | 调起系统截图工具框选，选完自动上传（可标注、可吸附窗口） |
| `Ctrl + Alt + S` | 全屏截图上传 |
| `Ctrl + Alt + C` | 上传剪贴板里当前的图片 |

命令行也全部支持，可以直接做快捷方式：

```bash
python mumu_shot.py --full                      # 全屏截图上传
python mumu_shot.py --region                    # 交互框选
python mumu_shot.py --region 100,100,800,600    # 固定区域
python mumu_shot.py --snip                      # 调系统截图工具
python mumu_shot.py --clipboard                 # 上传剪贴板图片
python mumu_shot.py --window MuMu               # 截取指定标题的窗口
python mumu_shot.py --list-windows              # 列出所有窗口标题
python mumu_shot.py --open-gallery              # 上传后顺手在模拟器里打开
python mumu_shot.py --device 127.0.0.1:16384    # 手动指定实例（多开时用）
```

## 端口是怎么自动找到的

各家模拟器的 adb 端口并不统一（实测 MuMu 6.8.4 用的是 **16384**，而不是常见的 7555），多开时每个实例还各不相同。所以这里不硬编码，而是按准确度依次尝试：

1. **问 MuMuManager** —— `MuMuManager info -v all` 直接给出每个实例的 `adb_port` 和 Android 是否已启动（最快也最准）
2. **扫进程监听端口** —— 没有 MuMuManager 的老版本，就读 `tasklist` 找出虚拟机进程 PID，再从 `netstat` 拿到它实际监听的本地端口
3. **常见端口兜底** —— 7555 / 16384 / 5555 等，最后一道保险

任何一步失败都会给出明确原因，而不是笼统地报"连接失败"。

## 常见问题

**首次运行弹 SmartScreen 警告？**
exe 没有代码签名，Windows 会拦一下。点「更多信息」→「仍要运行」，之后不再提示。

**上传失败去哪看原因？**
exe 同目录下的 `mumu-tool.log`（UTF-8，记事本直接打开），每次连接了哪个实例、文件的 media id 都记在里面。

**热键按了没反应？**
可能被别的软件占用了。面板底部会直接告诉你哪几个被占用，用面板按钮一样能截图。

**模拟器多开怎么办？**
自动选第一个已启动的实例。想指定具体实例用 `--device 127.0.0.1:<端口>`。

**会往模拟器里塞垃圾吗？**
只往 `/sdcard/Pictures/MuMuShot/` 一个目录写，在图库里单独成一个相簿，删起来干净。

## 项目结构

```
mumu_shot.py          截图工具主程序（热键、面板、四种截图方式）
mumu_core.py          MuMu 公共能力：adb 自动发现 / 推送 / 媒体库扫描
tools/
  make_icons.py       生成图标
  rebuild_exe.py      一键重新打包成 exe
  rebuild-exe.bat     Windows 下双击重新打包
  shot.ico/.png       图标
```

重新打包（改完代码后）：

```bash
pip install pyinstaller pillow
python tools/rebuild_exe.py
```

## 环境要求

- Windows 10 / 11
- MuMu 模拟器（已在 MuMu 6.8.4 / Android 15 上实测通过；其他模拟器只要能 adb 连接，改一下识别规则也能用）

## License

[MIT](LICENSE)
