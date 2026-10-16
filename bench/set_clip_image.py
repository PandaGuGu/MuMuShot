"""把一张纯色小图放进 Windows 剪贴板（CF_DIB），用于验证 --clipboard 取图路径。"""
import ctypes
import struct

u32 = ctypes.windll.user32
k32 = ctypes.windll.kernel32

k32.GlobalAlloc.argtypes = [ctypes.c_uint, ctypes.c_size_t]
k32.GlobalAlloc.restype = ctypes.c_void_p
k32.GlobalLock.argtypes = [ctypes.c_void_p]
k32.GlobalLock.restype = ctypes.c_void_p
k32.GlobalUnlock.argtypes = [ctypes.c_void_p]
u32.OpenClipboard.argtypes = [ctypes.c_void_p]
u32.EmptyClipboard.restype = ctypes.c_int
u32.SetClipboardData.argtypes = [ctypes.c_uint, ctypes.c_void_p]
u32.SetClipboardData.restype = ctypes.c_void_p
u32.CloseClipboard.restype = ctypes.c_int

W, H = 64, 48
header = struct.pack("<IiiHHIIiiII", 40, W, H, 1, 32, 0, W * H * 4, 0, 0, 0, 0)
pixels = b"\x33\x33\xcc\xff" * (W * H)  # BGRA：偏红
dib = header + pixels

GMEM_MOVEABLE = 0x0002
CF_DIB = 8

h = k32.GlobalAlloc(GMEM_MOVEABLE, len(dib))
ptr = k32.GlobalLock(h)
ctypes.memmove(ptr, dib, len(dib))
k32.GlobalUnlock(h)

u32.OpenClipboard(None)
u32.EmptyClipboard()
ok = u32.SetClipboardData(CF_DIB, h)
u32.CloseClipboard()
print("set clipboard CF_DIB:", bool(ok), "size:", W, "x", H)
