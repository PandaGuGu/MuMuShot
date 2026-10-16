#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""重新打包 MuMuShot（Python 版）。

改完 mumu_shot.py / mumu_core.py 之后跑一下这个脚本，就会重新打包出
可双击运行的 MuMuShot.exe，并把 exe + _internal 放回项目根目录。

目录约定（源码在 python/，跑起来的程序在项目根目录）：

    MuMuShot/
    ├── python/                  ← 源码（本脚本所在位置）
    │   ├── mumu_shot.py
    │   ├── mumu_core.py
    │   └── tools/
    ├── MuMuShot.exe             ← 打包产物（本脚本的落地点）
    └── _internal/

注意：**只替换 MuMuShot.exe 和 _internal 这两项**，同目录下的源码、.git、
docs、rust 等一概不动。想改落地点就设环境变量 MUMUSHOT_DEST。
"""

import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.dirname(HERE)              # python/（mumu_shot.py 所在目录）
OUT = os.path.dirname(SRC)               # 项目根目录
BUILD_DIR = os.path.join(SRC, "dist_dir")

# 打包产物的落地点：默认项目根目录（用户习惯双击的那份），可用环境变量覆盖
DEST_DIR = os.environ.get("MUMUSHOT_DEST") or OUT

NAME = "MuMuShot"
SOURCE = "mumu_shot.py"
ICON = "shot.ico"
EXE_NAME = "MuMuShot.exe"
INTERNAL = "_internal"
EXCLUDES = ["cv2", "numpy", "matplotlib", "scipy", "pandas"]


def build() -> bool:
    cmd = [sys.executable, "-m", "PyInstaller",
           "--noconfirm", "--clean", "--onedir", "--noconsole",
           "--name", NAME,
           "--icon", os.path.join("tools", ICON),
           "--distpath", "dist_dir",
           "--workpath", "build_dir",
           "--specpath", "."]
    for mod in EXCLUDES:
        cmd += ["--exclude-module", mod]
    cmd.append(SOURCE)
    print(f"\n>>> 打包 {NAME} …", flush=True)
    r = subprocess.run(cmd, cwd=SRC, capture_output=True)
    tail = (r.stdout + r.stderr).decode("utf-8", "replace").strip().splitlines()[-3:]
    print("\n".join("    " + t for t in tail), flush=True)
    return r.returncode == 0


def publish() -> bool:
    """把产物里的 exe 与 _internal 换到落地点，其余文件一律不碰。"""
    built = os.path.join(BUILD_DIR, NAME)
    src_exe = os.path.join(built, EXE_NAME)
    src_internal = os.path.join(built, INTERNAL)

    if not os.path.isfile(src_exe):
        print(f"    [x] 没找到产物 {src_exe}")
        return False

    os.makedirs(DEST_DIR, exist_ok=True)

    dst_exe = os.path.join(DEST_DIR, EXE_NAME)
    dst_internal = os.path.join(DEST_DIR, INTERNAL)

    # 只清掉旧的 exe 与 _internal，别的（源码 / .git / docs …）一律不动
    if os.path.isfile(dst_exe):
        os.remove(dst_exe)
    if os.path.isdir(dst_internal):
        shutil.rmtree(dst_internal, ignore_errors=True)

    shutil.copy2(src_exe, dst_exe)
    if os.path.isdir(src_internal):
        shutil.copytree(src_internal, dst_internal)

    print(f"    [√] {dst_exe}")
    print(f"    [√] {os.path.join(dst_internal, '*')}")
    return True


def cleanup() -> None:
    for junk in ("build_dir", "dist_dir", "build", "dist"):
        path = os.path.join(SRC, junk)
        if os.path.isdir(path):
            shutil.rmtree(path, ignore_errors=True)
    for f in os.listdir(SRC):
        if f.endswith(".spec"):
            os.remove(os.path.join(SRC, f))
    shutil.rmtree(os.path.join(SRC, "__pycache__"), ignore_errors=True)


def main():
    ok = build() and publish()
    cleanup()
    print("\n完成。" if ok else "\n有一步失败了，看上面的输出。")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
