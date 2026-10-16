#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""MuMu 安卓模拟器的 adb 公共能力。

被 mumu_shot.py（截图投送）复用，也可以单独调用：

    import mumu_core
    adb = mumu_core.find_adb()
    serial = mumu_core.pick_device(adb)              # 自动找到模拟器并连上
    mumu_core.push_photo(adb, serial, "a.png",
                        "/sdcard/Pictures/MuMuShot", False)
"""

from __future__ import annotations


import ctypes


import datetime as dt


import glob


import json


import os


import re


import shutil


import subprocess


import sys


import time


MUMU_ROOTS = [
    r"F:\Program Files\Netease\MuMu",
    r"C:\Program Files\Netease\MuMu",
    r"C:\Program Files (x86)\Netease\MuMu",
    r"D:\Program Files\Netease\MuMu",
    r"E:\Program Files\Netease\MuMu",
]


DEVICE_CANDIDATES = ["127.0.0.1:7555", "127.0.0.1:16384", "127.0.0.1:5555", "emulator-5554"]


CREATE_NO_WINDOW = 0x08000000


MUMU_VM_PROC_HINTS = ("device", "player", "headless", "nemu")


ADB_LIKELY_PORTS = {"7555", "16384", "5555", "21503", "21513", "62001", "62025", "62026", "5554"}


HERE = os.path.dirname(os.path.abspath(__file__))


def app_dir() -> str:
    """打包成 exe 后取 exe 所在目录，脚本运行时取脚本所在目录。"""
    if getattr(sys, "frozen", False):
        return os.path.dirname(os.path.abspath(sys.executable))
    return HERE


APP_DIR = app_dir()


LOG_PATH = os.path.join(APP_DIR, "mumu-tool.log")


LAST_ERROR = ""


LAST_DEVICE_DESC = ""


def log(msg: str = "") -> None:
    """同时写到控制台和日志文件（打包成无控制台的 exe 后就只剩日志文件了）。"""
    text = str(msg)
    try:
        if sys.stdout is not None:
            print(text, flush=True)
    except Exception:
        pass
    try:
        if os.path.exists(LOG_PATH) and os.path.getsize(LOG_PATH) > 256 * 1024:
            os.replace(LOG_PATH, LOG_PATH + ".old")
        with open(LOG_PATH, "a", encoding="utf-8") as f:
            f.write(f"{dt.datetime.now():%m-%d %H:%M:%S}  {text}\n")
    except Exception:
        pass


def find_adb(explicit: str | None = None) -> str | None:
    """按 MuMu 自带 adb → 环境变量 adb 的顺序找一个可用的 adb。"""
    if explicit:
        return explicit if os.path.exists(explicit) else None

    cands: list[str] = []
    for root in MUMU_ROOTS:
        cands += sorted(glob.glob(os.path.join(root, "nx_device", "*", "shell", "adb.exe")), reverse=True)
        cands.append(os.path.join(root, "nx_main", "adb.exe"))
        cands.append(os.path.join(root, "shell", "adb.exe"))
    for c in cands:
        if os.path.exists(c):
            return c
    return shutil.which("adb")


def run(cmd: list[str], timeout: int = 30, binary: bool = False):
    """执行命令，返回 (returncode, 文本输出)。binary=True 时返回原始 bytes。

    打包成窗口版 exe 后自身没有控制台，必须加 CREATE_NO_WINDOW，
    否则每次调 adb 都会闪出一个黑色命令行窗口。
    """
    kwargs = {"creationflags": CREATE_NO_WINDOW} if os.name == "nt" else {}
    try:
        p = subprocess.run(cmd, capture_output=True, timeout=timeout, **kwargs)
    except subprocess.TimeoutExpired:
        return 124, b"timeout" if binary else "timeout"
    raw = p.stdout + p.stderr
    if binary:
        return p.returncode, raw
    return p.returncode, raw.decode("utf-8", "replace")


def find_mumu_manager() -> str | None:
    for root in MUMU_ROOTS:
        p = os.path.join(root, "nx_main", "MuMuManager.exe")
        if os.path.exists(p):
            return p
    return None


def mumu_instances() -> list[dict]:
    """问 MuMuManager 要所有实例的信息（adb 端口、Android 是否已启动）。

    比硬编码端口靠谱得多：MuMu 各版本的 adb 端口并不固定（实测这台机器是 16384，
    而不是常见的 7555），多开时每个实例还各不相同。
    """
    mg = find_mumu_manager()
    if not mg:
        return []
    rc, out = run([mg, "info", "-v", "all"], timeout=20)
    start = out.find("{")
    if rc != 0 or start < 0:
        return []
    try:
        data = json.loads(out[start:])
    except Exception:
        return []
    if not isinstance(data, dict):
        return []
    return [v for v in data.values() if isinstance(v, dict) and v.get("adb_port")]


def mumu_ports_from_processes() -> list[str]:
    """没有任何配置可读时，直接看 MuMu 进程在监听哪些本地端口。

    这样即使 MuMuManager 不在（老版本）或者配置读不到，也不用去猜端口号。
    """
    rc, out = run(["tasklist", "/FO", "CSV", "/NH"], timeout=20)
    if rc != 0:
        return []

    vm_pids, other_pids = set(), set()
    for line in out.splitlines():
        parts = [p.strip().strip('"') for p in line.split('","')]
        if len(parts) < 2:
            continue
        name, pid = parts[0].strip('"'), parts[1].strip('"')
        low = name.lower()
        if not (low.startswith("mumu") or low.startswith("nemu")):
            continue
        if any(h in low for h in MUMU_VM_PROC_HINTS):
            vm_pids.add(pid)
        else:
            other_pids.add(pid)

    rc, out = run(["netstat", "-ano"], timeout=20)
    if rc != 0:
        return []

    def collect(pids: set[str]) -> list[str]:
        found = []
        for line in out.splitlines():
            f = line.split()
            if len(f) < 5 or f[0].upper() != "TCP" or f[3].upper() != "LISTENING":
                continue
            if f[4] not in pids:
                continue
            addr, _, port = f[1].rpartition(":")
            if not port.isdigit() or port == "0":
                continue
            if addr not in ("127.0.0.1", "0.0.0.0", "[::]", "::"):
                continue
            if port not in found:
                found.append(port)
        return found

    # 虚拟机进程持有的端口最可能是 adb，优先；找不到再看 MuMu 的其他进程
    vm_ports = collect(vm_pids)
    ports = vm_ports + [p for p in collect(other_pids) if p not in vm_ports]
    # 同类里先试"长得像 adb"的端口（各家模拟器的常用端口），
    # 免得先去连 MuMu 的渲染/控制端口、白等好几秒握手超时
    ports.sort(key=lambda p: (p not in ADB_LIKELY_PORTS,))
    return ports[:8]


def pick_device(adb: str, wanted: str | None = None) -> str | None:
    """自动找到正在运行的 MuMu 实例并连上它。

    识别顺序（越靠前越准确）：
      1. 显式指定的设备
      2. MuMuManager 报出的实例 adb 端口（最准，还能区分多开实例）
      3. 扫 MuMu 进程实际监听的本地端口（不依赖任何配置，也不猜端口号）
      4. 常见端口 / adb devices 兜底
    """
    global LAST_ERROR, LAST_DEVICE_DESC

    def try_serial(serial: str, boot_retry: int = 0) -> bool:
        if ":" in serial:
            # 连真实 adb 端口是毫秒级；连到非 adb 端口会等握手超时，所以别给太长
            run([adb, "connect", serial], timeout=6)
        for i in range(boot_retry + 1):
            rc, out = run([adb, "-s", serial, "shell", "getprop", "ro.build.version.release"],
                          timeout=12)
            low = out.lower()
            if rc == 0 and out.strip() and "error" not in low and "offline" not in low:
                return True
            if i < boot_retry:
                time.sleep(1.5)  # 模拟器正在开机，给它一点时间
        return False

    if wanted:
        if try_serial(wanted, boot_retry=3):
            LAST_DEVICE_DESC = f"{wanted}（手动指定）"
            return wanted
        LAST_ERROR = f"连不上指定的设备 {wanted}"
        return None

    instances = mumu_instances()

    # 1) MuMuManager 报的端口：已启动的实例优先，并容忍它还在开机
    ordered = sorted(instances, key=lambda i: (not i.get("is_android_started"),
                                               str(i.get("index", "0"))))
    for item in ordered:
        host = item.get("adb_host_ip") or "127.0.0.1"
        serial = f"{host}:{item['adb_port']}"
        if try_serial(serial, boot_retry=3 if item.get("is_android_started") else 0):
            idx = item.get("index")
            name = item.get("name") or "MuMu"
            LAST_DEVICE_DESC = f"{serial}（实例 {idx}「{name}」）"
            return serial

    # 2) 扫 MuMu 进程实际监听的端口
    for port in mumu_ports_from_processes():
        serial = f"127.0.0.1:{port}"
        if try_serial(serial):
            LAST_DEVICE_DESC = f"{serial}（从 MuMu 进程的监听端口里找到）"
            return serial

    # 3) MuMuManager 明确知道实例状态时，不用再猜常见端口了（省好几秒）
    if instances:
        LAST_ERROR = "MuMu 模拟器没在运行（或正在启动/关闭中），请先打开模拟器再试"
        return None

    # 4) 最后才试常见端口 / adb devices
    for serial in DEVICE_CANDIDATES:
        if try_serial(serial):
            LAST_DEVICE_DESC = serial
            return serial

    rc, out = run([adb, "devices"], timeout=15)
    for line in out.splitlines()[1:]:
        parts = line.split()
        if len(parts) >= 2 and parts[1] == "device":
            LAST_DEVICE_DESC = f"{parts[0]}（来自 adb devices）"
            return parts[0]

    LAST_ERROR = "没检测到运行中的 MuMu 实例，请先启动模拟器"
    return None


def adb_shell(adb: str, serial: str, args: list[str], timeout: int = 30):
    return run([adb, "-s", serial, "shell"] + args, timeout=timeout)


def scan_media(adb: str, serial: str) -> None:
    """触发 MediaStore 重新扫描存储卷，两种方式都试一遍。"""
    adb_shell(
        adb,
        serial,
        ["content", "call", "--uri", "content://media/external/file",
         "--method", "scan_volume", "--arg", "external_primary"],
        timeout=40,
    )


def query_media_id(adb: str, serial: str, remote_path: str) -> str | None:
    """在媒体库里查这张图的 _id，查到说明图库已经能看到它。

    注意：Windows 下 adb 转发命令会把单引号吃掉，所以不能带 --where 过滤，
    改成把图片列表拉回来在本地按文件名匹配。
    """
    name = os.path.basename(remote_path)
    rc, out = adb_shell(
        adb,
        serial,
        ["content", "query", "--uri", "content://media/external/images/media",
         "--projection", "_id:_data"],
        timeout=30,
    )
    for m in re.finditer(r"_id=(\d+),\s*_data=(\S+)", out):
        if m.group(2).endswith(name):
            return m.group(1)
    return None


def push_photo(adb: str, serial: str, local: str, remote_dir: str, open_gallery: bool) -> bool:
    """把照片推到模拟器相册目录，并让图库立即可见。"""
    global LAST_ERROR
    name = os.path.basename(local)
    remote = f"{remote_dir.rstrip('/')}/{name}"

    adb_shell(adb, serial, ["mkdir", "-p", remote_dir], timeout=20)

    rc, out = run([adb, "-s", serial, "push", local, remote], timeout=120)
    if rc != 0:
        LAST_ERROR = f"推送到模拟器失败：{out.strip()[:200]}"
        log(f"  [x] 推送失败：{out.strip()}")
        return False

    # 让所有进程都能读到
    adb_shell(adb, serial, ["chmod", "666", remote], timeout=20)
    scan_media(adb, serial)

    media_id = None
    for _ in range(6):
        media_id = query_media_id(adb, serial, remote)
        if media_id:
            break
        time.sleep(0.8)

    if media_id:
        LAST_ERROR = ""
        log(f"  [√] 已进入模拟器相册：{remote}  (media id={media_id})")
    else:
        log(f"  [!] 文件已推送，但媒体库暂未索引：{remote}（在图库里下拉刷新一下即可）")

    if open_gallery:
        if media_id:
            adb_shell(adb, serial, ["am", "start", "-a", "android.intent.action.VIEW",
                                    "-d", f"content://media/external/images/media/{media_id}",
                                    "-t", "image/*"], timeout=20)
        else:
            adb_shell(adb, serial, ["monkey", "-p", "com.android.gallery3d",
                                    "-c", "android.intent.category.LAUNCHER", "1"], timeout=20)
    return True

def show_error(msg: str, title: str = "MuMuShot") -> None:
    """没有控制台（打包成窗口版 exe）时，用弹框把错误告诉用户。"""
    log(f"  [x] {msg}")
    if sys.stdout is not None:
        return  # 有控制台，用户能直接看到
    try:
        ctypes.windll.user32.MessageBoxW(None, msg, title, 0x10)
    except Exception:
        pass


def ensure_dir(path: str) -> str:
    """能写就写在旁边，写不了（比如放在只读目录）就退到临时目录。"""
    try:
        os.makedirs(path, exist_ok=True)
        return path
    except Exception:
        import tempfile
        alt = os.path.join(tempfile.gettempdir(), "MuMuTool")
        os.makedirs(alt, exist_ok=True)
        return alt
