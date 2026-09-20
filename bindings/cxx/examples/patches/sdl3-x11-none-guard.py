#!/usr/bin/env python3
# MSRTC 补丁: SDL3 3.4.16 X11 events 对 Xwayland 传入的 None 原子裸解引用（真崩点）。
#
# 背景（09-20 出窗轮，librarian 对 release-3.4.16 与 main 逐字比对 = upstream 未修）：
# Xwayland（GNOME/mutter rootless）会话里 PropertyNotify / XdndTypeList / SDL_FORMATS
# property 可携带 atom=0(None)，SDL 多处 XGetAtomName 无 None 防护：
#   - 触发 Xlib 默认 error handler → exit(1)（SDL 无全局 handler）；
#   - 失败返 NULL 后 strlen/strncmp/stpcpy(NULL) → 二阶 segfault。
# 逐点加 None/NULL 守护（= 兄弟点 1436/1897 既有 `if (name)` 形，upstream 本应如此）。
# 否决 app 侧全局 XSetErrorHandler：会吞掉所有 X 错（含真错），劣于精准补。
#
# 由 examples/CMakeLists.txt FetchContent(sdl3) PATCH_COMMAND 应用；档案 zip 保持上游
# 逐字节（sha256 对账在解包前）。每点独立幂等（含 marker 则跳过）。
import sys

f = "src/video/x11/SDL_x11events.c"
try:
    c = open(f, encoding="utf-8").read()
except FileNotFoundError:
    sys.exit(0)  # FetchContent 工作目录非源根时静默（stamp 机制会重跑）

MARK = "MSRTC-PATCH-sdl3-x11-none"
subs = [
    # ① X11_PickTarget：Xwayland XdndTypeList 可含 None（兄弟函数 PickTargetFromAtoms 同款过滤）
    ("        char *name = X11_XGetAtomName(disp, list[i]);",
     "        if (list[i] == None) { continue; } // %s A: XdndTypeList None" % MARK + "\n"
     "        char *name = X11_XGetAtomName(disp, list[i]);"),
    # ②③ SDL_FORMATS 双循环：property 数据可含 0 → strlen/stpcpy(NULL)
    ("                char *atomStr = X11_XGetAtomName(display, *patom);\n"
     "                allocationsize += SDL_strlen(atomStr) + 1;",
     "                char *atomStr = X11_XGetAtomName(display, *patom);\n"
     "                allocationsize += (atomStr ? SDL_strlen(atomStr) : 4) + 1; // %s B" % MARK),
    ("                    char *atomStr = X11_XGetAtomName(display, *patom);\n"
     "                    new_mime_types[j] = strPtr;\n"
     "                    strPtr = stpcpy(strPtr, atomStr) + 1;",
     "                    char *atomStr = X11_XGetAtomName(display, *patom);\n"
     "                    new_mime_types[j] = strPtr;\n"
     "                    strPtr = stpcpy(strPtr, atomStr ? atomStr : \"None\") + 1; // %s C" % MARK),
    # ④ PropertyNotify：atom 可 None → strncmp(NULL)（同函数 1436 形已有 if(name) 守护，此为无守护分支）
    ("        char *name_of_atom = X11_XGetAtomName(display, xevent->xproperty.atom);\n\n"
     "        if (SDL_strncmp(name_of_atom,",
     "        char *name_of_atom = X11_XGetAtomName(display, xevent->xproperty.atom);\n\n"
     "        if (name_of_atom && SDL_strncmp(name_of_atom,"),
]

applied = 0
for old, new in subs:
    tag = new.split(MARK)[-1].strip().split('"')[0].strip().split("\n")[0][:1]
    if MARK + " " + tag in old and False:
        continue
    if MARK in new and MARK + " " + tag in c:
        continue  # 该点已打
    if MARK + " " + tag not in c and old in c:
        c = c.replace(old, new, 1)
        applied += 1

if applied:
    open(f, "w", encoding="utf-8").write(c)
    print(f"{MARK}: {applied} 点已应用")
else:
    print(f"{MARK}: 无需改动（幂等）")
