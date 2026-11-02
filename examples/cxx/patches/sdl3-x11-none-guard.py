#!/usr/bin/env python3
# MSRTC patch: SDL3 3.4.16's X11 events dereference None atoms passed by Xwayland without a guard (the real crash point).
#
# Background (the 09-20 window round; the librarian compared release-3.4.16 against main verbatim = unfixed upstream):
# In Xwayland (GNOME/mutter rootless) sessions, PropertyNotify / XdndTypeList / SDL_FORMATS
# properties can carry atom=0 (None), and several SDL XGetAtomName call sites lack None guards:
#   - it triggers the default Xlib error handler → exit(1) (SDL has no global handler);
#   - after a NULL return, strlen/strncmp/stpcpy(NULL) → a second-order segfault.
# Add None/NULL guards point by point (mirroring the existing `if (name)` form at sibling sites 1436/1897 — what upstream should have done).
# A global app-side XSetErrorHandler was rejected: it swallows all X errors (including real ones), inferior to a precise patch.
#
# Applied by examples/CMakeLists.txt's FetchContent(sdl3) PATCH_COMMAND; the archive zip stays byte-for-byte
# upstream (the sha256 check runs before extraction). Each site is independently idempotent (skipped when the marker is present).
import sys

f = "src/video/x11/SDL_x11events.c"
try:
    c = open(f, encoding="utf-8").read()
except FileNotFoundError:
    sys.exit(0)  # silent when the FetchContent workdir is not the source root (the stamp mechanism will re-run)

MARK = "MSRTC-PATCH-sdl3-x11-none"
subs = [
    # ① X11_PickTarget: Xwayland's XdndTypeList can contain None (the same filtering as the sibling PickTargetFromAtoms)
    ("        char *name = X11_XGetAtomName(disp, list[i]);",
     "        if (list[i] == None) { continue; } // %s A: XdndTypeList None" % MARK + "\n"
     "        char *name = X11_XGetAtomName(disp, list[i]);"),
    # ②③ The two SDL_FORMATS loops: property data can contain 0 → strlen/stpcpy(NULL)
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
    # ④ PropertyNotify: atom can be None → strncmp(NULL) (the same function's form at 1436 already has the if(name) guard; this is the unguarded branch)
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
        continue  # this site is already patched
    if MARK + " " + tag not in c and old in c:
        c = c.replace(old, new, 1)
        applied += 1

if applied:
    open(f, "w", encoding="utf-8").write(c)
    print(f"{MARK}: {applied} sites applied")
else:
    print(f"{MARK}: nothing to do (idempotent)")
