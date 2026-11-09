# cmake/gui-deps/source-mode.cmake -- the B8 archive-source policy, ONE point.
#
# Two consumers, one policy: build.cmake (the explicit SDL verb, script mode)
# and MediaServoGuiDeps.cmake (the configure-time imgui fetch) must resolve
# the SAME archive source the SAME way -- otherwise a P-mode configure could
# disagree with an S-mode build about which bytes were trusted.
#
#   mediaservo_gui_deps_source_mode(<repo_root> <out_local> <out_dl_args>)
#
# Sets <out_local> to the LOCAL_ARCHIVE_DIR value and <out_dl_args> to the
# extra polyorch_thirdparty_recipe arguments:
#   - the parent shell injected MSRTC_3RDPARTY_DIR -> P mode: the override
#     archive layer IS the source (zero network; the archives ship in the
#     parent repo, clone = full archive set, CI included);
#   - no env -> S mode: the caller allows ONE controlled fetch into
#     <repo>/.cache/3rdparty-archives (survives clean trees, r2-H6 -- never
#     derived from the build tree); later acquires replay from the cache
#     with the network dead.
# Strict-first is not relaxed anywhere: without this helper's DOWNLOAD
# arguments the PolyOrch loader never touches the network.
function(mediaservo_gui_deps_source_mode _msm_root _msm_out_local _msm_out_dl)
    set(_msm_env "$ENV{MSRTC_3RDPARTY_DIR}")
    if(_msm_env)
        set(${_msm_out_local} "${_msm_env}" PARENT_SCOPE)
        set(${_msm_out_dl} "" PARENT_SCOPE)
        message(STATUS "gui-deps source: override layer ${_msm_env} (MSRTC_3RDPARTY_DIR)")
    else()
        set(${_msm_out_local} "" PARENT_SCOPE)
        set(${_msm_out_dl} "DOWNLOAD;on;DOWNLOAD_CACHE_DIR;${_msm_root}/.cache/3rdparty-archives" PARENT_SCOPE)
        message(STATUS "gui-deps source: no override env -- one controlled download into ${_msm_root}/.cache/3rdparty-archives allowed")
    endif()
endfunction()
