#!/usr/bin/env python3
"""X11 helper for the REAPER scenarios that need the editor window (cargo xtask reaper-test; lane RH).

Talks to the display in $DISPLAY, which is always the Xvfb server `xvfb-run` started for REAPER: it refuses to run
anywhere else. Uses only the standard library (ctypes) with libX11 and libXtst, which Xvfb setups ship.

    xinput.py find <title-substring>         print "<window id> <x> <y> <w> <h>" of the top-level window
    xinput.py client <title-substring>       the same for its innermost (largest leaf) child: the plugin's own window
    xinput.py raise <title-substring>        stack the top-level window on top (there is no window manager)
    xinput.py click <x> <y>                  press and release button 1 at root coordinates
    xinput.py drag <x1> <y1> <x2> <y2>       press at 1, move in steps to 2, release
    xinput.py key <keysym-name>              press and release a key (e.g. z, Escape)
    xinput.py stats <title-substring>        raise the window, then print "<distinct colours> <non-background fraction>"
                                             of the client window's pixels (sampled)

Exit status 0 on success, 2 when the window is missing, 3 when it is not (yet) drawable, 1 on any other error.
"""
import ctypes
import ctypes.util
import os
import sys
import time

DISPLAY = os.environ.get("DISPLAY", "")
# xvfb-run's displays are :99 and up ("-a" picks a free one); the real display is :0 or :1 on this machine.
if not DISPLAY.startswith(":") or not DISPLAY[1:].split(".")[0].isdigit() or int(DISPLAY[1:].split(".")[0]) < 50:
    sys.exit("xinput: refusing to run on DISPLAY=%r: only under xvfb-run" % DISPLAY)

x11 = ctypes.CDLL(ctypes.util.find_library("X11") or "libX11.so.6")
xtst = ctypes.CDLL(ctypes.util.find_library("Xtst") or "libXtst.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
x11.XDefaultRootWindow.restype = ctypes.c_ulong
x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x11.XQueryTree.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong),
                           ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)), ctypes.POINTER(ctypes.c_uint)]
x11.XFree.argtypes = [ctypes.c_void_p]
x11.XFetchName.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_char_p)]
x11.XRaiseWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XFlush.argtypes = [ctypes.c_void_p]
x11.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
x11.XStringToKeysym.restype = ctypes.c_ulong
x11.XStringToKeysym.argtypes = [ctypes.c_char_p]
x11.XKeysymToKeycode.restype = ctypes.c_ubyte
x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x11.XTranslateCoordinates.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_int, ctypes.c_int,
                                      ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_ulong)]
xtst.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
xtst.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
xtst.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]


class Attributes(ctypes.Structure):
    _fields_ = [("x", ctypes.c_int), ("y", ctypes.c_int), ("width", ctypes.c_int), ("height", ctypes.c_int),
                ("border_width", ctypes.c_int), ("depth", ctypes.c_int), ("visual", ctypes.c_void_p),
                ("root", ctypes.c_ulong), ("klass", ctypes.c_int), ("bit_gravity", ctypes.c_int),
                ("win_gravity", ctypes.c_int), ("backing_store", ctypes.c_int), ("backing_planes", ctypes.c_ulong),
                ("backing_pixel", ctypes.c_ulong), ("save_under", ctypes.c_int), ("colormap", ctypes.c_ulong),
                ("map_installed", ctypes.c_int), ("map_state", ctypes.c_int), ("all_event_masks", ctypes.c_long),
                ("your_event_mask", ctypes.c_long), ("do_not_propagate_mask", ctypes.c_long),
                ("override_redirect", ctypes.c_int), ("screen", ctypes.c_void_p)]


class Image(ctypes.Structure):
    _fields_ = [("width", ctypes.c_int), ("height", ctypes.c_int), ("xoffset", ctypes.c_int), ("format", ctypes.c_int),
                ("data", ctypes.POINTER(ctypes.c_ubyte)), ("byte_order", ctypes.c_int), ("bitmap_unit", ctypes.c_int),
                ("bitmap_bit_order", ctypes.c_int), ("bitmap_pad", ctypes.c_int), ("depth", ctypes.c_int),
                ("bytes_per_line", ctypes.c_int), ("bits_per_pixel", ctypes.c_int)]


x11.XGetWindowAttributes.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(Attributes)]
x11.XGetImage.restype = ctypes.POINTER(Image)
x11.XGetImage.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint,
                          ctypes.c_ulong, ctypes.c_int]

# An X error (a window that went away between two calls, a read from an unviewable window) must not print a dump and
# kill the helper in the middle of a scenario: remember it and let the command fail with a status.
X_ERRORS = []
ERROR_HANDLER = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p)(lambda display, event: X_ERRORS.append(1) or 0)
x11.XSetErrorHandler.argtypes = [ctypes.c_void_p]
x11.XSetErrorHandler(ERROR_HANDLER)

dpy = x11.XOpenDisplay(None)
if not dpy:
    sys.exit("xinput: cannot open display %s" % DISPLAY)
root = x11.XDefaultRootWindow(dpy)


def children(window):
    r, p = ctypes.c_ulong(), ctypes.c_ulong()
    kids, n = ctypes.POINTER(ctypes.c_ulong)(), ctypes.c_uint()
    if not x11.XQueryTree(dpy, window, ctypes.byref(r), ctypes.byref(p), ctypes.byref(kids), ctypes.byref(n)):
        return []
    out = [kids[i] for i in range(n.value)]
    if kids:
        x11.XFree(kids)
    return out


def name(window):
    text = ctypes.c_char_p()
    if x11.XFetchName(dpy, window, ctypes.byref(text)) and text.value:
        return text.value.decode("utf-8", "replace")
    return ""


def geometry(window):
    """(x, y, w, h) in root coordinates, and whether the window is viewable."""
    a = Attributes()
    x11.XGetWindowAttributes(dpy, window, ctypes.byref(a))
    ox, oy, child = ctypes.c_int(), ctypes.c_int(), ctypes.c_ulong()
    x11.XTranslateCoordinates(dpy, window, root, 0, 0, ctypes.byref(ox), ctypes.byref(oy), ctypes.byref(child))
    return ox.value, oy.value, a.width, a.height, a.map_state == 2


def find(substring):
    for window in children(root):
        if substring in name(window):
            return window
    sys.exit(2)


def leaf(window):
    """The largest descendant that has no children: the plugin's own window inside REAPER's frame."""
    kids = children(window)
    if not kids:
        return window
    best = max(kids, key=lambda k: geometry(k)[2] * geometry(k)[3])
    return leaf(best)


def pause(seconds=0.02):
    x11.XFlush(dpy)
    time.sleep(seconds)


def move(x, y):
    xtst.XTestFakeMotionEvent(dpy, -1, int(x), int(y), 0)
    pause()


def button(down):
    xtst.XTestFakeButtonEvent(dpy, 1, 1 if down else 0, 0)
    pause()


def main():
    args = sys.argv[1:]
    cmd = args[0] if args else ""
    if cmd in ("find", "client"):
        window = find(args[1])
        if cmd == "client":
            window = leaf(window)
        x, y, w, h, _ = geometry(window)
        print(window, x, y, w, h)
    elif cmd == "raise":
        x11.XRaiseWindow(dpy, find(args[1]))
        pause(0.1)
    elif cmd == "click":
        move(args[1], args[2])
        pause(0.05)
        button(True)
        pause(0.05)
        button(False)
    elif cmd == "drag":
        x1, y1, x2, y2 = (float(a) for a in args[1:5])
        move(x1, y1)
        pause(0.05)
        button(True)
        for i in range(1, 11):
            move(x1 + (x2 - x1) * i / 10, y1 + (y2 - y1) * i / 10)
        pause(0.05)
        button(False)
    elif cmd == "key":
        code = x11.XKeysymToKeycode(dpy, x11.XStringToKeysym(args[1].encode()))
        xtst.XTestFakeKeyEvent(dpy, code, 1, 0)
        pause(0.05)
        xtst.XTestFakeKeyEvent(dpy, code, 0, 0)
        pause()
    elif cmd == "stats":
        top = find(args[1])
        window = leaf(top)
        x, y, w, h, viewable = geometry(window)
        if not viewable or w < 16 or h < 16:
            sys.exit(3)
        # A window under another one reads back the other one's pixels, so stack it first and give it a moment to
        # repaint the exposed part.
        x11.XRaiseWindow(dpy, top)
        pause(0.15)
        img = x11.XGetImage(dpy, window, 0, 0, w, h, 0xFFFFFFFF, 2)
        x11.XSync(dpy, 0)
        if not img or X_ERRORS:
            sys.exit(3)
        im = img.contents
        stride, bpp = im.bytes_per_line, im.bits_per_pixel // 8
        raw = ctypes.string_at(im.data, stride * h)
        colours, counts = {}, 0
        step = 4
        for row in range(0, h, step):
            line = raw[row * stride:row * stride + w * bpp]
            for col in range(0, w, step):
                px = line[col * bpp:(col + 1) * bpp]
                colours[px] = colours.get(px, 0) + 1
                counts += 1
        background = max(colours.values())
        print(len(colours), "%.4f" % (1.0 - background / counts))
    else:
        sys.exit(__doc__)
    x11.XSync(dpy, 0)


main()
