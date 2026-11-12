# sdl3-x11-none-guard.cmake -- MSRTC patch: SDL3 3.4.16's X11 events
# dereference None atoms passed by Xwayland without a guard (the real
# crash point).
#
# Background (the 09-20 window round; the librarian compared
# release-3.4.16 against main verbatim = unfixed upstream):
# In Xwayland (GNOME/mutter rootless) sessions, PropertyNotify /
# XdndTypeList / SDL_FORMATS properties can carry atom=0 (None), and
# several SDL XGetAtomName call sites lack None guards:
#   - it triggers the default Xlib error handler -> exit(1) (SDL has no
#     global handler);
#   - after a NULL return, strlen/strncmp/stpcpy(NULL) -> a second-order
#     segfault.
# Add None/NULL guards point by point (mirroring the existing `if (name)`
# form at sibling sites 1436/1897 -- what upstream should have done).
# A global app-side XSetErrorHandler was rejected: it swallows all X
# errors (including real ones), inferior to a precise patch.
#
# Applied by the sdl3 recipe's patch_command (PolyOrch third-party
# face); the archive zip stays byte-for-byte upstream (the sha256 check
# runs before extraction). Each site is independently idempotent:
# sites A/B/C skip when their marker is present; site D carries no
# marker and is idempotent because its old pattern disappears after
# the patch.
#
# Run: cmake -P sdl3-x11-none-guard.cmake with CWD = the SDL3 source
# tree (the recipe loader resolves this script's path against the
# recipe base; the command text is part of the fetch stamp key).
#
# CMake port of sdl3-x11-none-guard.py (python-retirement micro-round,
# 11-12); behavior preserved (first-occurrence replacement, per-site
# idempotence, silent no-op outside the source root), fixture-verified.

set(_guard_file "src/video/x11/SDL_x11events.c")
if(NOT EXISTS "${_guard_file}")
  # Silent when the workdir is not the source root (the stamp mechanism
  # will re-run).
  return()
endif()
file(READ "${_guard_file}" _guard_c)
set(_guard_mark "MSRTC-PATCH-sdl3-x11-none")
set(_guard_applied 0)

# _guard_site(<old> <new> <tag>) -- replace the FIRST occurrence of
# <old> with <new> (python replace(old, new, 1) parity). A non-empty
# <tag> enables the marker skip (already-patched site); an empty tag
# relies on the old pattern disappearing.
function(_guard_site old new tag)
  if(NOT tag STREQUAL "")
    string(FIND "${_guard_c}" "${_guard_mark} ${tag}" _mpos)
    if(_mpos GREATER -1)
      return()
    endif()
  endif()
  string(FIND "${_guard_c}" "${old}" _pos)
  if(_pos EQUAL -1)
    return()
  endif()
  string(LENGTH "${old}" _olen)
  string(SUBSTRING "${_guard_c}" 0 ${_pos} _head)
  math(EXPR _tail "${_pos} + ${_olen}")
  string(SUBSTRING "${_guard_c}" ${_tail} -1 _rest)
  set(_guard_c "${_head}${new}${_rest}" PARENT_SCOPE)
  math(EXPR _n "${_guard_applied} + 1")
  set(_guard_applied "${_n}" PARENT_SCOPE)
endfunction()

# Site A -- X11_PickTarget: Xwayland's XdndTypeList can contain None
# (the same filtering as the sibling PickTargetFromAtoms).
_guard_site(
  "        char *name = X11_XGetAtomName(disp, list[i]);"
  "        if (list[i] == None) { continue; } // ${_guard_mark} A: XdndTypeList None\n        char *name = X11_XGetAtomName(disp, list[i]);"
  "A")

# Site B -- SDL_FORMATS size loop: property data can contain 0 ->
# strlen(NULL).
_guard_site(
  "                char *atomStr = X11_XGetAtomName(display, *patom);\n                allocationsize += SDL_strlen(atomStr) + 1;"
  "                char *atomStr = X11_XGetAtomName(display, *patom);\n                allocationsize += (atomStr ? SDL_strlen(atomStr) : 4) + 1; // ${_guard_mark} B"
  "B")

# Site C -- SDL_FORMATS mime loop: stpcpy(NULL).
_guard_site(
  "                    char *atomStr = X11_XGetAtomName(display, *patom);\n                    new_mime_types[j] = strPtr;\n                    strPtr = stpcpy(strPtr, atomStr) + 1;"
  "                    char *atomStr = X11_XGetAtomName(display, *patom);\n                    new_mime_types[j] = strPtr;\n                    strPtr = stpcpy(strPtr, atomStr ? atomStr : \"None\") + 1; // ${_guard_mark} C"
  "C")

# Site D -- PropertyNotify: atom can be None -> strncmp(NULL) (the
# same function's form at 1436 already has the if(name) guard; this is
# the unguarded branch). No marker: the old pattern disappears.
_guard_site(
  "        char *name_of_atom = X11_XGetAtomName(display, xevent->xproperty.atom);\n\n        if (SDL_strncmp(name_of_atom,"
  "        char *name_of_atom = X11_XGetAtomName(display, xevent->xproperty.atom);\n\n        if (name_of_atom && SDL_strncmp(name_of_atom,"
  "")

if(_guard_applied GREATER 0)
  file(WRITE "${_guard_file}" "${_guard_c}")
  message(STATUS "${_guard_mark}: ${_guard_applied} sites applied")
else()
  message(STATUS "${_guard_mark}: nothing to do (idempotent)")
endif()
