import ctypes
import time

u = ctypes.windll.user32

LEFTDOWN = 0x0002
LEFTUP = 0x0004

x0, y0 = 300, 300
x1, y1 = 1000, 760

u.SetCursorPos(x0, y0)
time.sleep(0.4)
u.mouse_event(LEFTDOWN, 0, 0, 0, 0)
time.sleep(0.25)
steps = 24
for i in range(1, steps + 1):
    x = x0 + int((x1 - x0) * i / steps)
    y = y0 + int((y1 - y0) * i / steps)
    u.SetCursorPos(x, y)
    time.sleep(0.035)
time.sleep(0.3)
u.mouse_event(LEFTUP, 0, 0, 0, 0)
print("drag done", x0, y0, x1, y1, "->", x1 - x0, "x", y1 - y0)
