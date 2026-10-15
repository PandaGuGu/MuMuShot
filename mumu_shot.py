#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
MuMuShot —— MuMu 截图助手
把电脑屏幕上的画面截下来，一键塞进 MuMu 模拟器的相册，供模拟器内的
微信 / 抖音 / 相册等 App 直接使用。

四种用法：
    1) 双击 mumu-shot.bat 打开小面板，按按钮截图
    2) 全局热键（不用切窗口）：
         Ctrl + Alt + A   框选一块区域截图上传（自带遮罩选区，最快）
         Ctrl + Alt + Q   调起系统截图工具（Win+Shift+S）框选后自动上传
         Ctrl + Alt + S   全屏截图上传
         Ctrl + Alt + C   上传剪贴板里当前的图片
    3) 命令行 / 快捷方式：
         python mumu_shot.py --full                      全屏截图上传
         python mumu_shot.py --region                    交互框选
         python mumu_shot.py --region 100,100,800,600    固定区域截图
         python mumu_shot.py --snip                      系统截图工具框选
         python mumu_shot.py --clipboard                 上传剪贴板里的图片
         python mumu_shot.py --window MuMu               截取指定标题的窗口
         python mumu_shot.py --list-windows              列出所有窗口标题
"""

from __future__ import annotations

import argparse
import ctypes
import datetime as dt
import os
import queue
import subprocess
import sys
import threading
import time
from ctypes import wintypes

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import mumu_core  # noqa: E402
from mumu_core import (APP_DIR, LOG_PATH, ensure_dir, find_adb, log,  # noqa: E402
                       pick_device, push_photo, show_error)

PHOTO_DIR = os.path.join(APP_DIR, "shots")
DEFAULT_REMOTE_DIR = "/sdcard/Pictures/MuMuShot"

HK_MOD_ALT = 0x0001
HK_MOD_CONTROL = 0x0002
HK_MOD_NOREPEAT = 0x4000
WM_HOTKEY = 0x0312

VK_S, VK_A, VK_Q, VK_C = 0x53, 0x41, 0x51, 0x43
VK_LWIN, VK_SHIFT = 0x5B, 0x10
KEYEVENTF_KEYUP = 0x0002


def fatal(msg: str) -> None:
    """打包后没有控制台，出错时弹个框告诉用户。"""
    show_error(msg, "MuMuShot - 出错了")


# ---------------------------------------------------------------- DPI / 屏幕
def enable_dpi_awareness() -> None:
    """让截图坐标和屏幕物理像素一一对应，避免缩放屏截歪。"""
    try:
        ctypes.windll.shcore.SetProcessDpiAwareness(2)  # Per-Monitor V2
        return
    except Exception:
        pass
    try:
        ctypes.windll.user32.SetProcessDPIAware()
    except Exception:
        pass


def virtual_screen_rect() -> tuple[int, int, int, int]:
    """整个虚拟桌面（含多显示器）的 (left, top, width, height)。"""
    u = ctypes.windll.user32
    left = u.GetSystemMetrics(76)    # SM_XVIRTUALSCREEN
    top = u.GetSystemMetrics(77)     # SM_YVIRTUALSCREEN
    width = u.GetSystemMetrics(78)   # SM_CXVIRTUALSCREEN
    height = u.GetSystemMetrics(79)  # SM_CYVIRTUALSCREEN
    if width <= 0 or height <= 0:    # 兜底
        width, height = u.GetSystemMetrics(0), u.GetSystemMetrics(1)
        left = top = 0
    return left, top, width, height


# ------------------------------------------------------------------- 窗口
class RECT(ctypes.Structure):
    _fields_ = [("left", wintypes.LONG), ("top", wintypes.LONG),
                ("right", wintypes.LONG), ("bottom", wintypes.LONG)]


def window_rect(hwnd: int) -> tuple[int, int, int, int] | None:
    """窗口的真实可见边界（去掉 Win10+ 的透明外框）。"""
    r = RECT()
    dwm = ctypes.windll.dwmapi
    if dwm.DwmGetWindowAttribute(wintypes.HWND(hwnd), 9, ctypes.byref(r), ctypes.sizeof(r)) != 0:
        if not ctypes.windll.user32.GetWindowRect(wintypes.HWND(hwnd), ctypes.byref(r)):
            return None
    if r.right - r.left <= 0 or r.bottom - r.top <= 0:
        return None
    return r.left, r.top, r.right, r.bottom


SKIP_WINDOW_CLASSES = {
    "Windows.UI.Core.CoreWindow",  # 输入法候选 / 系统浮层
    "ApplicationManager_DesktopShellWindow",
    "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Progman", "WorkerW",
}


def list_windows() -> list[tuple[int, str, tuple[int, int, int, int]]]:
    """所有可见且带标题的顶层窗口。"""
    user32 = ctypes.windll.user32
    result: list[tuple[int, str, tuple[int, int, int, int]]] = []
    EnumProc = ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)

    def cb(hwnd, _lparam):
        if not user32.IsWindowVisible(hwnd) or user32.IsIconic(hwnd):
            return True
        n = user32.GetWindowTextLengthW(hwnd)
        if n <= 0:
            return True
        buf = ctypes.create_unicode_buffer(n + 1)
        user32.GetWindowTextW(hwnd, buf, n + 1)
        title = buf.value.strip()
        if not title:
            return True
        if user32.GetWindowLongW(hwnd, -20) & 0x00000080:  # WS_EX_TOOLWINDOW
            return True
        cbuf = ctypes.create_unicode_buffer(256)
        user32.GetClassNameW(hwnd, cbuf, 256)
        if cbuf.value in SKIP_WINDOW_CLASSES:
            return True
        rect = window_rect(hwnd)
        if rect:
            result.append((hwnd, title, rect))
        return True

    user32.EnumWindows(EnumProc(cb), 0)
    # 面积大的排前面，通常就是你要的那个
    result.sort(key=lambda x: (x[2][2] - x[2][0]) * (x[2][3] - x[2][1]), reverse=True)
    return result


# ------------------------------------------------------------------- 截图
def grab_full():
    from PIL import ImageGrab
    return ImageGrab.grab(all_screens=True)


def grab_bbox(bbox):
    from PIL import ImageGrab
    return ImageGrab.grab(bbox=bbox, all_screens=True)


def grab_clipboard():
    from PIL import ImageGrab
    data = ImageGrab.grabclipboard()
    if data is None:
        return None
    if isinstance(data, list):  # 剪贴板里是文件路径（Win11 截图工具会这样给）
        from PIL import Image
        for p in data:
            if str(p).lower().endswith((".png", ".jpg", ".jpeg", ".bmp", ".webp", ".gif")):
                try:
                    return Image.open(p).convert("RGB")
                except Exception:
                    continue
        return None
    try:
        return data.convert("RGB")
    except Exception:
        return None


# ----------------------------------------------- 调起系统截图工具（Win+Shift+S）
def clipboard_seq() -> int:
    return int(ctypes.windll.user32.GetClipboardSequenceNumber())


def send_win_shift_s() -> None:
    u = ctypes.windll.user32
    for vk in (VK_LWIN, VK_SHIFT, VK_S):
        u.keybd_event(vk, 0, 0, 0)
    for vk in (VK_S, VK_SHIFT, VK_LWIN):
        u.keybd_event(vk, 0, KEYEVENTF_KEYUP, 0)


def trigger_snip() -> bool:
    """调起 Windows 截图工具。优先用 ms-screenclip: 协议，失败就模拟 Win+Shift+S。"""
    kwargs = {"creationflags": 0x08000000, "stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL}
    for cmd in (["explorer.exe", "ms-screenclip:"],):
        try:
            subprocess.Popen(cmd, **kwargs)
            return True
        except Exception:
            continue
    try:
        send_win_shift_s()
        return True
    except Exception:
        return False


def snip_via_system(timeout: float = 90.0):
    """让系统截图工具接管框选（可标注、可吸附窗口），完成后从剪贴板取图。"""
    seq0 = clipboard_seq()
    if not trigger_snip():
        log("  [x] 没能调起系统截图工具")
        return None
    log("  已调起系统截图工具，请框选要上传的区域（ESC 取消）…")

    t0 = time.time()
    while time.time() - t0 < timeout:
        time.sleep(0.25)
        if clipboard_seq() == seq0:
            continue
        # 剪贴板变了，等截图工具把文件也落盘
        time.sleep(0.3)
        for _ in range(10):
            img = grab_clipboard()
            if img is not None:
                return img
            time.sleep(0.3)
        time.sleep(0.4)
    log("  [!] 等待系统截图超时，已取消")
    return None


def select_region(root=None):
    """全屏变暗 + 拖拽框选，返回 (图像, bbox)。没有图形库时退回系统截图工具。"""
    try:
        import tkinter as tk
        from PIL import ImageEnhance, ImageTk
    except ImportError:
        log("  [!] 当前 Python 没有图形库，改用系统截图工具")
        return snip_via_system(), None

    vx, vy, vw, vh = virtual_screen_rect()
    full = grab_full()  # 先抓一张原图，最后裁剪用的就是它
    dim = ImageEnhance.Brightness(full.convert("RGB")).enhance(0.42)

    own_root = root is None
    if own_root:
        root = tk.Tk()
        root.withdraw()

    top = tk.Toplevel(root)
    top.overrideredirect(True)
    top.geometry(f"{vw}x{vh}+{vx}+{vy}")
    top.attributes("-topmost", True)
    top.configure(bg="black")

    canvas = tk.Canvas(top, width=vw, height=vh, highlightthickness=0, cursor="crosshair")
    canvas.pack()
    bg = ImageTk.PhotoImage(dim)
    canvas.img_ref = bg  # 保住引用，否则被回收后会报 image ... doesn't exist
    canvas.create_image(0, 0, anchor="nw", image=bg)

    state: dict = {"bbox": None}

    def on_press(e):
        state["x0"], state["y0"] = e.x, e.y
        if "rect" in state:
            canvas.delete(state["rect"])
        state["rect"] = canvas.create_rectangle(e.x, e.y, e.x, e.y, outline="#31e07a", width=2)
        state["size"] = canvas.create_text(e.x + 8, e.y - 14, anchor="nw", fill="#31e07a",
                                           font=("Segoe UI", 11), text="")

    def on_drag(e):
        if "x0" not in state:
            return
        canvas.coords(state["rect"], state["x0"], state["y0"], e.x, e.y)
        w, h = abs(e.x - state["x0"]), abs(e.y - state["y0"])
        canvas.coords(state["size"], min(e.x, state["x0"]) + 8, min(e.y, state["y0"]) + 8)
        canvas.itemconfig(state["size"], text=f"{w} × {h}")
        canvas.tag_raise(state["size"])

    def on_release(e):
        if "x0" not in state:
            top.destroy()
            return
        x0, y0 = state["x0"], state["y0"]
        sx0, sy0 = min(x0, e.x), min(y0, e.y)
        sx1, sy1 = max(x0, e.x), max(y0, e.y)
        state["bbox"] = (sx0, sy0, sx1, sy1) if (sx1 - sx0) >= 4 and (sy1 - sy0) >= 4 else None
        top.destroy()

    def cancel(_e=None):
        state["bbox"] = None
        top.destroy()

    canvas.bind("<ButtonPress-1>", on_press)
    canvas.bind("<B1-Motion>", on_drag)
    canvas.bind("<ButtonRelease-1>", on_release)
    top.bind("<Escape>", cancel)
    top.bind("<Button-3>", cancel)
    canvas.focus_set()
    top.focus_force()
    top.grab_set()

    root.wait_window(top)

    bbox = state["bbox"]
    if own_root:
        try:
            root.destroy()
        except Exception:
            pass
    if not bbox:
        return None, None
    crop = full.crop((bbox[0], bbox[1], bbox[2], bbox[3]))
    abs_bbox = (vx + bbox[0], vy + bbox[1], vx + bbox[2], vy + bbox[3])
    return crop, abs_bbox


def grab_window(keyword: str):
    wins = list_windows()
    if not wins:
        return None, None
    kw = keyword.lower()
    hit = next((w for w in wins if kw in w[1].lower()), None)
    if not hit:
        return None, None
    return grab_bbox(hit[2]), hit[2]


# ------------------------------------------------------------- 全局热键
class Hotkeys(threading.Thread):
    """在工作线程里注册系统级热键，触发后把动作名丢进队列。"""

    def __init__(self, bindings, out_queue: queue.Queue):
        super().__init__(daemon=True)
        self.bindings = bindings
        self.out = out_queue
        self.registered: list[str] = []
        self.failed: list[str] = []

    def run(self):
        user32 = ctypes.windll.user32
        ids = {}
        for i, (mods, vk, name) in enumerate(self.bindings, start=1):
            if user32.RegisterHotKey(None, i, mods, vk):
                ids[i] = name
                self.registered.append(name)
            else:
                self.failed.append(name)
        if not ids:
            return
        msg = wintypes.MSG()
        while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) != 0:
            if msg.message == WM_HOTKEY:
                name = ids.get(int(msg.wParam))
                if name:
                    self.out.put(name)
        for i in ids:
            user32.UnregisterHotKey(None, i)


HOTKEY_LIST = [
    (HK_MOD_CONTROL | HK_MOD_ALT | HK_MOD_NOREPEAT, VK_A, "region", "Ctrl+Alt+A  框选截图"),
    (HK_MOD_CONTROL | HK_MOD_ALT | HK_MOD_NOREPEAT, VK_Q, "snip", "Ctrl+Alt+Q  系统截图工具框选"),
    (HK_MOD_CONTROL | HK_MOD_ALT | HK_MOD_NOREPEAT, VK_S, "full", "Ctrl+Alt+S  全屏截图"),
    (HK_MOD_CONTROL | HK_MOD_ALT | HK_MOD_NOREPEAT, VK_C, "clip", "Ctrl+Alt+C  上传剪贴板图片"),
]


# --------------------------------------------------------------- 上传
class Uploader:
    """负责把本地图片送进 MuMu 相册，连接信息只解析一次。"""

    def __init__(self, args):
        self.args = args
        self.adb = None
        self.serial = None
        self.count = 0
        self.last_error = ""
        self.out_dir = ensure_dir(PHOTO_DIR)

    def connect(self) -> bool:
        if self.serial:
            return True
        adb = find_adb(getattr(self.args, "adb", None))
        if not adb:
            self.last_error = "没找到 adb，请确认已安装 MuMu 模拟器"
            log(f"  [x] {self.last_error}")
            return False
        serial = pick_device(adb, getattr(self.args, "device", None))
        if not serial:
            self.last_error = mumu_core.LAST_ERROR or "没连上 MuMu，请确认模拟器已经启动"
            log(f"  [x] {self.last_error}")
            return False
        self.adb, self.serial = adb, serial
        log(f"  [√] 已连上 MuMu：{mumu_core.LAST_DEVICE_DESC or serial}")
        return True

    def send(self, image, tag: str = "Shot", open_gallery: bool = False) -> bool:
        if image is None:
            self.last_error = "没拿到图像"
            log(f"  [x] {self.last_error}")
            return False
        if not self.connect():
            return False

        self.count += 1
        name = f"{tag}_{dt.datetime.now():%Y%m%d_%H%M%S}.png"
        local = os.path.join(self.out_dir, name)
        image.save(local, "PNG")
        log(f"[第 {self.count} 张] {local}  ({image.width}×{image.height})")
        ok = push_photo(self.adb, self.serial, local, self.args.remote_dir, open_gallery)
        self.last_error = "" if ok else (mumu_core.LAST_ERROR or "推送失败，详见日志文件")
        return ok


def capture_by_action(action: str, args):
    """按动作名产出图像，返回 (图像, 标签)。"""
    if action == "region":
        img, _ = select_region()
        return img, "Region"
    if action == "snip":
        return snip_via_system(getattr(args, "snip_timeout", 90.0)), "Snip"
    if action == "clip":
        return grab_clipboard(), "Clip"
    return grab_full(), "Shot"


# ---------------------------------------------------------------- GUI
def run_gui(args) -> int:
    import tkinter as tk
    from tkinter import ttk

    root = tk.Tk()
    root.title("MuMuShot 截图助手")
    root.geometry("410x400")
    root.minsize(390, 380)
    root.attributes("-topmost", True)

    def on_tk_error(exc, val, tb):
        """tkinter 回调里抛的异常默认只打到 stderr，打包后根本看不见，这里改成写日志。"""
        import traceback
        log(f"  [x] 界面出错：{val}\n{traceback.format_exc()}")

    root.report_callback_exception = on_tk_error

    up = Uploader(args)
    hk_queue: queue.Queue = queue.Queue()
    ui_queue: queue.Queue = queue.Queue()
    status = tk.StringVar(value="就绪")
    lock = threading.Lock()

    def post(fn, *a):
        """从任何线程把界面操作排队到主线程执行 —— tkinter 不是线程安全的。"""
        ui_queue.put((fn, a))

    def set_status(text: str):
        post(status.set, text)

    def drain_ui():
        try:
            while True:
                fn, a = ui_queue.get_nowait()
                fn(*a)
        except queue.Empty:
            pass
        root.after(60, drain_ui)

    def upload_async(img, tag, note: str = ""):
        """上传要跑 adb，放后台线程，别卡住面板。"""
        set_status(f"已截图{note} {img.width}×{img.height}，上传中…")

        def job():
            try:
                ok = up.send(img, tag, args.open_gallery)
                set_status(f"已上传到模拟器相册{note}（{img.width}×{img.height}）" if ok
                           else f"上传失败：{up.last_error}")
            except Exception as e:
                set_status(f"出错：{e}")
                log(f"  [x] {e}")
            finally:
                lock.release()

        threading.Thread(target=job, daemon=True).start()

    def work(action: str):
        """统一入口：截图 → 上传。

        关键点：框选要新建 tkinter 窗口，必须留在主线程执行；
        一旦丢进子线程，tkinter 会直接报 image "pyimageN" doesn't exist。
        """
        if not lock.acquire(blocking=False):
            set_status("上一张还在处理，稍等…")
            return

        if args.hide_panel_on_shot:
            root.withdraw()

        if action == "region":
            img = None
            try:
                if args.hide_panel_on_shot:
                    time.sleep(0.2)
                img, _ = select_region(root)
            except Exception as e:
                log(f"  [x] {e}")
                set_status(f"截图出错：{e}")
            finally:
                if args.hide_panel_on_shot:
                    root.deiconify()
                root.lift()
            if img is None:
                set_status("已取消框选")
                lock.release()
                return
            upload_async(img, "Region")
            return

        # 其余方式不碰 tkinter，可以安心丢到后台线程
        def job():
            try:
                if args.hide_panel_on_shot:
                    time.sleep(0.2)
                img, tag = capture_by_action(action, args)
            except Exception as e:
                log(f"  [x] {e}")
                set_status(f"出错：{e}")
                lock.release()
                return
            finally:
                if args.hide_panel_on_shot:
                    post(root.deiconify)
                post(root.lift)
            if img is None:
                set_status("已取消")
                lock.release()
                return
            upload_async(img, tag)

        threading.Thread(target=job, daemon=True).start()

    def work_window():
        wins = [w for w in list_windows() if w[1] != root.title()]
        if not wins:
            set_status("没找到可截取的窗口")
            return
        dlg = tk.Toplevel(root)
        dlg.title("选择窗口")
        dlg.geometry("560x360")
        dlg.attributes("-topmost", True)
        dlg.transient(root)
        tk.Label(dlg, text="双击要截取的窗口：").pack(anchor="w", padx=10, pady=(8, 4))
        lb = tk.Listbox(dlg, font=("Microsoft YaHei UI", 10))
        for _, title, rect in wins:
            lb.insert("end", f"{title}    ({rect[2] - rect[0]}×{rect[3] - rect[1]})")
        lb.pack(fill="both", expand=True, padx=10, pady=4)

        def take(_e=None):
            sel = lb.curselection()
            if not sel:
                return
            _hwnd, title, rect = wins[sel[0]]
            dlg.destroy()
            if not lock.acquire(blocking=False):
                return
            if args.hide_panel_on_shot:
                root.withdraw()

            def job():
                try:
                    from PIL import ImageGrab
                    img = ImageGrab.grab(bbox=rect, all_screens=True)
                except Exception as e:
                    log(f"  [x] {e}")
                    set_status(f"出错：{e}")
                    lock.release()
                    return
                finally:
                    if args.hide_panel_on_shot:
                        post(root.deiconify)
                    post(root.lift)
                upload_async(img, "Win", f"「{title}」")

            threading.Thread(target=job, daemon=True).start()

        lb.bind("<Double-Button-1>", take)
        tk.Button(dlg, text="截取这个窗口", command=take).pack(pady=8)

    frm = ttk.Frame(root, padding=14)
    frm.pack(fill="both", expand=True)

    ttk.Label(frm, text="截电脑屏幕 → 送进 MuMu 相册",
              font=("Microsoft YaHei UI", 12, "bold")).pack(anchor="w", pady=(0, 10))

    def big(text, cmd):
        ttk.Button(frm, text=text, command=cmd).pack(fill="x", pady=3, ipady=6)

    big("框选截图上传              Ctrl + Alt + A", lambda: work("region"))
    big("调系统截图工具框选        Ctrl + Alt + Q", lambda: work("snip"))
    big("全屏截图上传              Ctrl + Alt + S", lambda: work("full"))
    big("上传剪贴板里的图片        Ctrl + Alt + C", lambda: work("clip"))
    big("截取指定窗口", work_window)

    v1 = tk.BooleanVar(value=args.open_gallery)
    v2 = tk.BooleanVar(value=args.hide_panel_on_shot)
    opts = ttk.Frame(frm)
    opts.pack(fill="x", pady=(10, 0))
    ttk.Checkbutton(opts, text="上传后自动在模拟器里打开图片", variable=v1,
                    command=lambda: setattr(args, "open_gallery", v1.get())).pack(anchor="w")
    ttk.Checkbutton(opts, text="截图瞬间隐藏本面板（避免拍进去）", variable=v2,
                    command=lambda: setattr(args, "hide_panel_on_shot", v2.get())).pack(anchor="w")

    tk.Label(frm, textvariable=status, wraplength=370, justify="left",
             fg="#7bd88f", font=("Microsoft YaHei UI", 9)).pack(anchor="w", pady=(10, 0))
    tk.Label(frm, text=f"照片同时留一份在本机：{PHOTO_DIR}", wraplength=370, justify="left",
             fg="#8a8a8a", font=("Microsoft YaHei UI", 8)).pack(anchor="w", pady=(4, 0))

    hk = Hotkeys([b[:3] for b in HOTKEY_LIST], hk_queue)
    hk.start()
    time.sleep(0.35)
    if hk.failed:
        names = "、".join({"region": "框选", "snip": "系统截图", "full": "全屏", "clip": "剪贴板"}
                          .get(n, n) for n in hk.failed)
        log(f"  [!] 这些热键被别的软件占用了：{names}")
        set_status(f"这些热键被别的软件占用了：{names}（用面板按钮即可）")
    elif hk.registered:
        log(f"  [√] 热键已注册：{'、'.join(hk.registered)}")
        set_status("就绪 · 热键 Ctrl+Alt+A/Q/S/C")

    def poll():
        try:
            while True:
                work(hk_queue.get_nowait())
        except queue.Empty:
            pass
        root.after(120, poll)

    root.after(60, drain_ui)
    root.after(120, poll)
    root.protocol("WM_DELETE_WINDOW", root.destroy)
    log("面板已打开：Ctrl+Alt+A 框选 / Q 系统截图 / S 全屏 / C 剪贴板，关掉窗口即退出。")
    root.mainloop()
    return 0


def run_console(args) -> int:
    """没有图形库时的降级方案：留一个控制台窗口，热键照常用。"""
    up = Uploader(args)
    hk_queue: queue.Queue = queue.Queue()
    hk = Hotkeys([b[:3] for b in HOTKEY_LIST], hk_queue)
    hk.start()
    time.sleep(0.35)

    log("MuMuShot（控制台模式）")
    for mods, vk, name, desc in HOTKEY_LIST:
        mark = "√" if name in hk.registered else "×"
        log(f"  [{mark}] {desc}")
    log("  这个窗口别关，关掉热键就失效了。")
    log("")

    while True:
        try:
            action = hk_queue.get()
        except KeyboardInterrupt:
            break
        try:
            img, tag = capture_by_action(action, args)
            up.send(img, tag, args.open_gallery)
        except Exception as e:
            log(f"  [x] {e}")
    return 0


# --------------------------------------------------------------- CLI
def selftest() -> int:
    """自检：确认依赖都在、目录可写、日志能落盘。

    打包成 exe 之后 CI 会先跑一遍这个，过不了就不发布 —— 免得发出去一个
    起不来的程序。
    """
    ok = True
    log(f"自检开始：Python {sys.version.split()[0]}，程序目录 {APP_DIR}")
    for mod in ("tkinter", "PIL", "PIL.ImageGrab", "PIL.ImageTk"):
        try:
            __import__(mod)
            log(f"  [√] {mod}")
        except Exception as e:
            log(f"  [x] {mod} 导入失败：{e}")
            ok = False
    try:
        import tkinter
        probe = tkinter.Tk()
        probe.withdraw()
        probe.destroy()
        log("  [√] 能创建 tkinter 窗口")
    except Exception as e:
        log(f"  [!] 创建窗口失败（无桌面环境下属正常）：{e}")
    log("自检通过" if ok else "自检失败")
    # 给 CI 留一个纯 ASCII 标记，避免日志匹配受编码影响
    log("selftest ok" if ok else "selftest failed")
    return 0 if ok else 1


def run_cli(args) -> int:
    up = Uploader(args)
    if not up.connect():
        return 3

    if args.region is not None and args.region not in ("", "select"):
        try:
            x0, y0, x1, y1 = (int(v) for v in args.region.replace(" ", "").split(","))
        except ValueError:
            log("  [x] --region 格式应为 x0,y0,x1,y1，或直接写 --region 表示交互框选")
            return 2
        img, tag = grab_bbox((x0, y0, x1, y1)), "Region"
    elif args.region is not None:
        img, tag = select_region()[0], "Region"
    elif args.snip:
        img, tag = snip_via_system(args.snip_timeout), "Snip"
    elif args.clipboard:
        img, tag = grab_clipboard(), "Clip"
    elif args.window is not None:
        img, rect = grab_window(args.window)
        if img is None:
            log(f"  [x] 没找到标题包含「{args.window}」的窗口，用 --list-windows 看看")
            return 5
        tag = "Win"
    else:
        img, tag = grab_full(), "Shot"

    return 0 if up.send(img, tag, args.open_gallery) else 4


def main() -> int:
    enable_dpi_awareness()

    ap = argparse.ArgumentParser(description="截取电脑屏幕并上传到 MuMu 模拟器相册")
    ap.add_argument("--full", action="store_true", help="全屏截图上传（默认动作）")
    ap.add_argument("--region", nargs="?", const="select", default=None,
                    metavar="x0,y0,x1,y1", help="框选截图；给坐标表示固定区域，如 100,100,800,600")
    ap.add_argument("--snip", action="store_true", help="调起系统截图工具框选后上传")
    ap.add_argument("--snip-timeout", type=float, default=90.0, help="等待系统截图完成的秒数，默认 90")
    ap.add_argument("--clipboard", action="store_true", help="上传剪贴板里的图片")
    ap.add_argument("--window", metavar="关键词", help="截取标题包含该关键词的窗口")
    ap.add_argument("--list-windows", action="store_true", help="列出所有窗口标题")
    ap.add_argument("--console", action="store_true", help="强制用控制台模式（无图形面板）")
    ap.add_argument("--selftest", action="store_true", help="自检：验证依赖与日志功能后退出")
    ap.add_argument("--open-gallery", action="store_true", help="上传后自动在模拟器里打开图片")
    ap.add_argument("--hide-panel-on-shot", action="store_true", help="截图瞬间隐藏面板")
    ap.add_argument("--remote-dir", default=DEFAULT_REMOTE_DIR,
                    help=f"模拟器相册目录，默认 {DEFAULT_REMOTE_DIR}")
    ap.add_argument("--adb", default=None, help="手动指定 adb.exe 路径")
    ap.add_argument("--device", default=None, help="手动指定设备，如 127.0.0.1:7555")
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    if args.list_windows:
        for hwnd, title, rect in list_windows():
            log(f"  {rect[2] - rect[0]:>5}×{rect[3] - rect[1]:<5}  {title}")
        return 0

    headless = args.full or args.clipboard or args.snip or args.window or (args.region is not None)
    if headless:
        return run_cli(args)

    if not args.console:
        try:
            import tkinter  # noqa: F401
            return run_gui(args)
        except ImportError:
            log("  [!] 当前 Python 没有图形库，改用控制台模式")
    return run_console(args)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SystemExit:
        raise
    except Exception as exc:  # 打包后没有控制台，兜底弹框
        import traceback
        fatal(f"{exc}\n\n{traceback.format_exc()}")
        sys.exit(1)
