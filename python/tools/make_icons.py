#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""生成两个工具图标（shot.ico 截图 / selfie.ico 自拍）。"""

from PIL import Image, ImageDraw

S = 1024  # 先画大图再缩小，边缘更干净
SIZES = [(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (16, 16)]

BG = (12, 30, 56)
GREEN = (49, 224, 122)
TEAL = (93, 202, 165)
LIGHT = (181, 212, 244)


def rounded(draw, box, r, fill, outline=None, width=0):
    draw.rounded_rectangle(box, radius=r, fill=fill, outline=outline, width=width)


def make_shot() -> Image.Image:
    """框选截图：深底 + 绿色选区框 + 四角把手 + 十字准星。"""
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    rounded(d, (40, 40, S - 40, S - 40), 190, BG)
    # 选区框
    m, w = 260, 22
    d.rectangle((m, m, S - m, S - m), outline=GREEN, width=w)
    # 四角把手
    hl = 110
    for cx, cy, dx, dy in ((m, m, 1, 1), (S - m, m, -1, 1), (m, S - m, 1, -1), (S - m, S - m, -1, -1)):
        d.line((cx, cy, cx + hl * dx, cy), fill=GREEN, width=w + 16)
        d.line((cx, cy, cx, cy + hl * dy), fill=GREEN, width=w + 16)
    # 十字准星
    c = S // 2
    d.line((c - 120, c, c + 120, c), fill=(255, 255, 255, 170), width=14)
    d.line((c, c - 120, c, c + 120), fill=(255, 255, 255, 170), width=14)
    return img


def make_selfie() -> Image.Image:
    """自拍：深底 + 相机机身 + 镜头。"""
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    rounded(d, (40, 40, S - 40, S - 40), 190, BG)
    # 机身
    rounded(d, (170, 300, S - 170, S - 250), 60, TEAL)
    # 取景器凸起
    rounded(d, (330, 230, 560, 320), 40, TEAL)
    # 镜头
    c = (S // 2, 570)
    d.ellipse((c[0] - 200, c[1] - 200, c[0] + 200, c[1] + 200), fill=BG)
    d.ellipse((c[0] - 175, c[1] - 175, c[0] + 175, c[1] + 175), outline=LIGHT, width=26)
    d.ellipse((c[0] - 95, c[1] - 95, c[0] + 95, c[1] + 95), fill=(40, 110, 170))
    d.ellipse((c[0] - 45, c[1] - 60, c[0] + 25, c[1] + 10), fill=(255, 255, 255, 200))
    # 闪光灯
    d.ellipse((S - 330, 350, S - 250, 430), fill=(255, 214, 120))
    return img


def main():
    import os
    out = os.path.dirname(os.path.abspath(__file__))
    for name, maker in (("shot.ico", make_shot), ("selfie.ico", make_selfie)):
        big = maker()
        big.save(os.path.join(out, name), format="ICO", sizes=SIZES)
        big.resize((256, 256), Image.LANCZOS).save(os.path.join(out, name.replace(".ico", ".png")))
        print(f"  生成 {name}")


if __name__ == "__main__":
    main()
