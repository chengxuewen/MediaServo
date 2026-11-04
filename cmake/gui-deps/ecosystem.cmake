# cmake/gui-deps/ecosystem.cmake -- the dev-only ecosystem intake leg (sr8 P7c).
#
# Same explicit-verb discipline as build.cmake (H2): script mode, one entry
# (./mediaservo.sh build gui-deps --ecosystem <vcpkg|conan> --pkg <name>
# [--generate-ecosystem]). It exists to prove the P7a/P7b lanes work from a
# real consumer tree, against a library nobody ships today (fmt). It is NOT a
# route for SDL3/imgui: SDL3 carries a patch and imgui a branch, so both stay
# on provider "source" (O-13/M5), and no default path changes here.
#
# Required: -DECOSYSTEM=vcpkg|conan -DPKG=<name> -DBUILD_TYPE=<cfg> -DWORK_DIR=<anchor>
# Optional: -DGENERATE=ON            the one network step (strict-first otherwise)
#           -DPACKAGES_DIR=<dir>     defaults to <repo>/.cache/3rdparty-ecosystem
#           -DCOMPILER_CXX=<abs>     the conan profile needs a stated compiler
cmake_minimum_required(VERSION 3.24)
get_filename_component(_ec_repo "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)
include("${_ec_repo}/PolyOrch/cmake/PolyOrchThirdPartyHelpers.cmake")

if(NOT ECOSYSTEM OR NOT PKG OR NOT BUILD_TYPE OR NOT WORK_DIR)
    message(FATAL_ERROR "gui-deps ecosystem: ECOSYSTEM, PKG, BUILD_TYPE and WORK_DIR are required (script mode has no defaults)")
endif()
if(NOT ECOSYSTEM MATCHES "^(vcpkg|conan)$")
    message(FATAL_ERROR "gui-deps ecosystem: ECOSYSTEM must be vcpkg|conan, got '${ECOSYSTEM}'")
endif()

# Two different axes, one flag each (r7 H1): PROVIDER/--provider is the
# BUILD-time dependency axis (what the child configure sees in its env),
# ECOSYSTEM/--ecosystem is the ACQUIRE axis (where the library comes from).
# The ecosystem lane builds inside the tool's own process, so the build-time
# axis has nothing to inject here -- accepting both would let one flag look
# like the other and quietly do nothing.
if(PROVIDER AND NOT PROVIDER STREQUAL "none")
    message(FATAL_ERROR "gui-deps ecosystem: --provider='${PROVIDER}' is the build-time dependency axis and does not apply to the ecosystem lane (the library builds inside ${ECOSYSTEM}); drop one of the two flags")
endif()

set(_ec_recipe "${CMAKE_CURRENT_LIST_DIR}/recipes/${PKG}-${ECOSYSTEM}.json")
if(NOT EXISTS "${_ec_recipe}")
    message(FATAL_ERROR "gui-deps ecosystem: no recipe for pkg='${PKG}' ecosystem='${ECOSYSTEM}' at ${_ec_recipe} (expected recipes/<pkg>-<ecosystem>.json, each pointing at the shared pin island in pkg-deps/<pkg>/)")
endif()

# The pack cache outlives the build anchor on purpose: the whole point of
# strict-first is generate once, then reuse offline, and an anchor-local cache
# would be deleted by the next clean. .cache/ is ignored and survives.
if(NOT PACKAGES_DIR)
    set(PACKAGES_DIR "${_ec_repo}/.cache/3rdparty-ecosystem")
endif()
file(MAKE_DIRECTORY "${PACKAGES_DIR}")

set(_ec_gen "")
if(ECOSYSTEM STREQUAL "conan")
    # The conan profile hash RIDES THE KEY (O-12=C: conan artifacts depend on the
    # compiler ABI), so the compiler fact is needed to compute an identity --
    # not only to generate. Requiring it only under GENERATE would make the
    # strict refusal die on the profile guard instead of printing its manual
    # (that is exactly what the first run of this leg did).
    # Script mode carries no CMAKE_CXX_* facts; the F25 posture is the system
    # pair, and this face never picks a compiler behind the caller's back.
    set(_ec_cxx "${COMPILER_CXX}")
    if(NOT _ec_cxx)
        set(_ec_cxx "/usr/bin/c++")
    endif()
    if(NOT EXISTS "${_ec_cxx}")
        message(FATAL_ERROR "gui-deps ecosystem: the conan profile needs a C++ compiler and '${_ec_cxx}' is absent -- pass COMPILER_CXX=<abs>")
    endif()
    set(PolyOrch_CONAN_COMPILER "${_ec_cxx}")
    message(STATUS "gui-deps ecosystem: conan profile compiler=${_ec_cxx} (stated, never guessed)")
endif()
if(GENERATE)
    set(_ec_gen GENERATE_ECOSYSTEM)
endif()

polyorch_thirdparty_recipe(RECIPE "${_ec_recipe}"
    WORK_DIR "${WORK_DIR}" BUILD_TYPE "${BUILD_TYPE}"
    PACKAGES_DIR "${PACKAGES_DIR}"
    ${_ec_gen}
    INSTALL_DIR_OUT _ec_install FINGERPRINT_OUT _ec_key MANIFEST_OUT _ec_mf
    FIND_PACKAGE_NAME_OUT _ec_fpn TARGETS_OUT _ec_tgts ALIAS_OUT _ec_alias)

# Layout is probed, never hardcoded (sr8 §2.8): the vcpkg prefix carries its
# config under share/<pkg>, the conan prefix puts the CMakeDeps configs at the
# top level beside direct_deploy/. Both are find_package-able from the prefix
# root, which is the only promise this leg asserts.
set(_ec_cfg "")
foreach(_cand "${_ec_install}/share/${PKG}/${PKG}-config.cmake"
              "${_ec_install}/share/${PKG}/find${PKG}-config.cmake"
              "${_ec_install}/${PKG}-config.cmake"
              "${_ec_install}/${PKG}Config.cmake")
    if(EXISTS "${_cand}")
        set(_ec_cfg "${_cand}")
        break()
    endif()
endforeach()
if(NOT _ec_cfg)
    message(FATAL_ERROR "gui-deps ecosystem: ${PKG} generated no CMake config reachable under ${_ec_install} -- the upstream install layout changed (looked for share/${PKG}/ and the prefix root)")
endif()
message(STATUS "gui-deps ecosystem: ${PKG}/${ECOSYSTEM} ${BUILD_TYPE} ready at ${_ec_install} (key ${_ec_key})")
message(STATUS "gui-deps ecosystem: config ${_ec_cfg}; wiring find_package=${_ec_fpn} targets=${_ec_tgts} alias=${_ec_alias}")
message(STATUS "gui-deps ecosystem: packs in ${PACKAGES_DIR} -- rerun without --generate-ecosystem to reuse offline")
