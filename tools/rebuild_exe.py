#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""重新打包 MuMuShot。

改完 mumu_shot.py / mumu_core.py 之后跑一下这个脚本，
就会重新生成可双击运行的「MuMuShot」文件夹，并同步到桌面目录。
"""

import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.dirname(HERE)          # mumu-selfie
OUT = os.path.dirname(SRC)           # 工作区根目录
BUILD_DIR = os.path.join(SRC, "dist_dir")

# 桌面上的投递位置（存在就同步过去）
MIRROR_DIR = r"C:\Users\Administrator\Desktop\项目集合\MuMuShot"
# 同步时保留这些（是用户数据，不能被覆盖掉）
MIRROR_KEEP = {"shots", "photos", "mumu-tool.log"}

NAME = "MuMuShot"
SOURCE = "mumu_shot.py"
ICON = "shot.ico"
FOLDER = "MuMuShot"
EXE_NAME = "MuMuShot.exe"
EXCLUDES = ["cv2", "numpy", "matplotlib", "scipy", "pandas"]


def build() -> bool:
    cmd = [sys.executable, "-m", "PyInstaller",
           "--noconfirm", "--onedir", "--noconsole",
           "--name", NAME,
           "--icon", os.path.join("tools", ICON),
           "--distpath", "dist_dir"]
    for mod in EXCLUDES:
        cmd += ["--exclude-module", mod]
    cmd.append(SOURCE)
    print(f"\n>>> 打包 {NAME} …", flush=True)
    r = subprocess.run(cmd, cwd=SRC, capture_output=True)
    tail = (r.stdout + r.stderr).decode("utf-8", "replace").strip().splitlines()[-3:]
    print("\n".join("    " + t for t in tail), flush=True)
    return r.returncode == 0


def mirror(final: str) -> None:
    """同步到桌面目录：换掉程序文件，保留截图和日志。"""
    if not os.path.isdir(MIRROR_DIR):
        return
    for item in os.listdir(MIRROR_DIR):
        if item in MIRROR_KEEP:
            continue
        path = os.path.join(MIRROR_DIR, item)
        if os.path.isdir(path):
            shutil.rmtree(path, ignore_errors=True)
        else:
            os.remove(path)
    for item in os.listdir(final):
        src, dst = os.path.join(final, item), os.path.join(MIRROR_DIR, item)
        if os.path.isdir(src):
            shutil.copytree(src, dst, dirs_exist_ok=True)
        else:
            shutil.copy2(src, dst)
    print(f"    [√] 已同步到 {MIRROR_DIR}")


def publish() -> bool:
    built = os.path.join(BUILD_DIR, NAME)
    if not os.path.isdir(built):
        print(f"    [x] 没找到产物 {built}")
        return False
    final = os.path.join(OUT, FOLDER)
    if os.path.isdir(final):
        shutil.rmtree(final)
    shutil.move(built, final)
    old_exe = os.path.join(final, NAME + ".exe")
    if os.path.exists(old_exe) and NAME + ".exe" != EXE_NAME:
        os.rename(old_exe, os.path.join(final, EXE_NAME))
    print(f"    [√] {final}\\{EXE_NAME}")
    mirror(final)
    return True


def main():
    ok = build() and publish()

    for junk in ("build", "dist_dir", "dist"):
        path = os.path.join(SRC, junk)
        if os.path.isdir(path):
            shutil.rmtree(path, ignore_errors=True)
    for f in os.listdir(SRC):
        if f.endswith(".spec"):
            os.remove(os.path.join(SRC, f))
    shutil.rmtree(os.path.join(SRC, "__pycache__"), ignore_errors=True)

    print("\n完成。" if ok else "\n有一步失败了，看上面的输出。")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
