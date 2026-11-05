# examples/py — python band of the top-level examples/ aggregator
# (examples-ide-unification ruling 4: the debug surface goes through the
# PolyOrch python face — SCRIPT-based, no executable target needed. The
# CLI `build example` py lane (`exec python3 examples/py/<name>.py`) stays
# the run path; this face adds the F5 rows and IDE group mounting.)
#
# Gate layering (measured, examples-ide-unification): the HOOK (end-of-
# configure generator) can only be mounted by a CACHE-level knob visible at
# PolyOrch embed time — the bridge cfgargs and the all-examples preset
# inject PolyOrch_VSCODE_DEBUG_ALL=ON for exactly that; a local set() can
# never retro-mount an include-time hook (MediaServoCrates:110's local
# set(ALL ON) is dead code for the same reason, recorded). The registration
# gate, however, is read AT the polyorch_python_run call — a plain
# directory-scope set() here is the same shape MediaServoCrates uses, and
# it is what makes the python specs register.

cmake_minimum_required(VERSION 3.24)
project(mediaservo_examples_py LANGUAGES NONE)

get_filename_component(REPO_ROOT "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)

# registry: one script per example — the loop IS the disk<->table audit
# (a missing script FATALs, same discipline as the c/rs band registries).
# Target names carry a py- band prefix: the rs band's registry already owns
# mediaservo-example-vehicle_push (a real artifact target, G4-locked), and
# its -run custom target would collide with a bare-name py target (CMP0002,
# measured). SCRIPT-based faces have no artifact, so the prefix is free.
set(_py_examples record_playback vehicle_push vehicle_signal)

set(PolyOrch_VSCODE_DEBUG_PYTHON ON)

foreach(_n IN LISTS _py_examples)
    if(NOT EXISTS "${CMAKE_CURRENT_LIST_DIR}/${_n}.py")
        message(FATAL_ERROR "examples/py registry entry '${_n}' has no script on disk")
    endif()
    if(COMMAND polyorch_python_run)
        polyorch_python_run(TARGET mediaservo-example-py-${_n}
            SCRIPT "${CMAKE_CURRENT_LIST_DIR}/${_n}.py"
            WORKING_DIRECTORY "${REPO_ROOT}"
            ENVS "MEDIASERVO_LIB_DIR=${REPO_ROOT}/target/debug"
            FOLDER "${MEDIASERVO_IDE_ROOT}examples/py")
    endif()
endforeach()

message(STATUS "examples py band: ${_py_examples} (F5 rows via the python face, ruling 4)")
