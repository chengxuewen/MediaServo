# MediaServoAppTree.cmake — SR7: out/{host,server} app delivery layout as install rules
#
# Contract: parent-repo docs/plans/polyorch-sr7-app-tree/PLAN.md (F32-C ruling).
#   * One leg one owner: the branded bin placement python's deploy bodies hand-laid
#     is THIS file now (deletion stroke same commit). Stateful steps (stop, init
#     render, identity, oxmgr vendor copy) are NOT placement and stay in the CLI.
#   * Route (ruling N2, verified): polyorch_rust_install DOES accept BINARY-kind
#     handles (supplier pass 1 rejects only rlib handles; the planner stages kind
#     bin into bin/ with exec perms) — but it has no per-row RENAME, and the app
#     tree's physical names ARE the brand (D269 brand = physical name). Mediator
#     rows would land upstream names. Landed route: plain install(PROGRAMS ...
#     RENAME ...) rows + consumer install(CODE) reclaim/link legs.
#   * Gate (cache-var, plan §T0): MEDIASERVO_APP_TREE "" | host | server. An empty
#     value registers ZERO rules — every preset/IDE/gates configure stays
#     behavior-neutral and no lane tree can re-lay an app tree (deploy server never
#     touches the SDK bindings tree and vice versa: the install invocation is
#     scoped to the preset tree whose gate matches the verb).
#   * Brand = cache variable (rule 3): the CLI resolves it (env MEDIASERVO_BRAND →
#     installed-instance derivation) and injects -DMEDIASERVO_BRAND; default empty
#     = upstream names, byte-stable. The SDK face stays brand-free (C47) — app
#     trees are the ONLY branded cmake output.
#   * Server rows source the delivery tree (out/server/bin) through the same
#     ordered hunt python used (BLOCKER-1 dual source: upstream name → brand name)
#     so the PIT-191 protection (build stages final names into the live out/ tree)
#     keeps feeding deploy. Weaknet adds the documented target/{release,debug}
#     fallback + absent = WARN-skip polarity [bF4/bF5].
#   * f8f85db rule: every install(CODE) leg below is script-mode policy-neutral
#     (list(FIND), never IN_LIST) — install scripts ignore the project's policy
#     floor and must not depend on CMP0057.
#   * Sweep legs are the whitelist-reclaim equivalents (deploy-ops 4): patterns are
#     the names python matched; the keep manifest is embedded at configure time
#     from THIS tree's brand (co-located installs never rely on each other —
#     oxmgr/caddy provably never match the *-server/*-weaknet/-<role> patterns).

set(MEDIASERVO_APP_TREE "" CACHE STRING "app delivery layout to describe at install time: '' (none) | host | server")
set(MEDIASERVO_BRAND "" CACHE STRING "product brand for the physical bin names (D269); empty = upstream names byte-stable")
set(MEDIASERVO_SERVER_OUT_DIR "" CACHE PATH "server delivery root (the CLI-resolved <out>/server) — server placement rows source from here")

# The eight host-tree bin names as cargo spells them (the CLI's HOST_BINS mirror —
# D269 names: host-<app> -> <brand>-<app>, mediaservo-host -> <brand>-host).
set(MEDIASERVO_HOST_BINS mediaservo-host host-agent host-streamer host-capturer
    host-recorder host-controller host-emergency host-audio)

function(mediaservo_setup_app_tree)
    if(MEDIASERVO_APP_TREE STREQUAL "server")
        if(NOT MEDIASERVO_SERVER_OUT_DIR)
            message(FATAL_ERROR
                "MEDIASERVO_APP_TREE=server requires -DMEDIASERVO_SERVER_OUT_DIR=<out>/server "
                "(the CLI resolves the delivery root — MSRTC_OUT_ROOT aware, PIT-189 family)")
        endif()
        _mediaservo_app_tree_server()
    elseif(MEDIASERVO_APP_TREE STREQUAL "host")
        _mediaservo_app_tree_host()
    else()
        message(FATAL_ERROR
            "MEDIASERVO_APP_TREE='${MEDIASERVO_APP_TREE}' is not one of ''|host|server")
    endif()
endfunction()

# -- server tree: bin/{brand}-server + bin/{brand}-weaknet + reclaim + entry links --
function(_mediaservo_app_tree_server)
    set(_exe "${CMAKE_EXECUTABLE_SUFFIX}")
    if(MEDIASERVO_BRAND)
        set(_sname "${MEDIASERVO_BRAND}-server${_exe}")
        set(_wname "${MEDIASERVO_BRAND}-weaknet${_exe}")
        set(_link_names "${MEDIASERVO_BRAND}-server")
    else()
        set(_sname "mediaservo-server${_exe}")
        set(_wname "mediaservo-weaknet${_exe}")
        set(_link_names "server;mediaservo-server")
    endif()

    # BLOCKER-1 dual-source hunt (ordered): upstream name (fresh build) -> brand
    # name (already-migrated tree: a second deploy needs no rebuild). Missing in
    # out/ = deploy precondition failure (D266: deploy does not trigger builds) —
    # same message the CLI used to print before its copy loop.
    set(_candidates "${MEDIASERVO_SERVER_OUT_DIR}/bin/mediaservo-server${_exe}")
    if(MEDIASERVO_BRAND)
        list(APPEND _candidates "${MEDIASERVO_SERVER_OUT_DIR}/bin/${_sname}")
    endif()
    set(_src "")
    foreach(_c IN LISTS _candidates)
        if(NOT _src AND EXISTS "${_c}")
            set(_src "${_c}")
        endif()
    endforeach()
    if(NOT _src)
        list(JOIN _candidates " / " _cand_text)
        message(FATAL_ERROR
            "Error: none of ${_cand_text} exists — run build server first "
            "(deploy does not trigger builds — D266)")
    endif()
    # In-place assembly (build:deploy server, prefix = the source tree): a
    # same-name landing is a file(INSTALL) no-op (measured on this cmake) — the
    # upstream->brand migration becomes copy + sweep reclaim (final bytes
    # identical; the transient double name is documented in the SR7 commit).
    install(PROGRAMS "${_src}" DESTINATION bin RENAME "${_sname}")

    # weaknet dual source + target/ fallback + absent = WARN skip ([bF4 bF5] —
    # never blocks the server main flow; the oxfile conditional-render predicate
    # belongs to Rust init, nailed by M-1 — cmake only settles the data plane).
    set(_wcands "${MEDIASERVO_SERVER_OUT_DIR}/bin/mediaservo-weaknet${_exe}")
    if(MEDIASERVO_BRAND)
        list(APPEND _wcands "${MEDIASERVO_SERVER_OUT_DIR}/bin/${_wname}")
    endif()
    list(APPEND _wcands
        "${MEDIASERVO_REPO_ROOT}/target/release/mediaservo-weaknet${_exe}"
        "${MEDIASERVO_REPO_ROOT}/target/debug/mediaservo-weaknet${_exe}")
    set(_wsrc "")
    foreach(_c IN LISTS _wcands)
        if(NOT _wsrc AND EXISTS "${_c}")
            set(_wsrc "${_c}")
        endif()
    endforeach()
    if(_wsrc)
        install(PROGRAMS "${_wsrc}" DESTINATION bin RENAME "${_wname}")
    else()
        message(WARNING
            "weaknet binary absent from all locations (out/server/bin and target/) — "
            "bin/${_wname} is NOT staged this time (does not block deploy; run build server)")
    endif()

    # bin whitelist reclaim (deploy-ops 4): delete *-server / *-weaknet / the
    # upstream server name that are not in THIS layout's keep manifest. oxmgr and
    # caddy never match the patterns (python predicate mirrored; the keep list is
    # brand-derived, not landed-derived — weaknet absent this run must NOT evict a
    # previously staged weaknet, exactly python's {oxmgr, bin, wnet-name} keep set).
    set(_keep_rx "${_sname}|${_wname}|oxmgr${_exe}|caddy${_exe}")
    install(CODE "
        set(_keepRx [==[${_keep_rx}]==])
        file(GLOB _binFiles \"\${CMAKE_INSTALL_PREFIX}/bin/*\")
        foreach(_f IN LISTS _binFiles)
            if(IS_SYMLINK \"\${_f}\")
                continue()
            endif()
            get_filename_component(_n \"\${_f}\" NAME)
            if(_n MATCHES \"^(\${_keepRx})\$\")
                continue()
            endif()
            if(_n MATCHES \"-server${_exe}$\" OR _n MATCHES \"-weaknet${_exe}$\"
               OR _n STREQUAL \"mediaservo-server${_exe}\")
                message(STATUS \"  cleaning deployment residue: \${_n}\")
                file(REMOVE \"\${_f}\")
            endif()
        endforeach()
    ")

    # root entry shortcuts (MED4 shape): relative symlinks into bin/ keep the
    # prefix relocatable; a filesystem without symlink support falls back to a
    # copy (python parity). Old-brand root links pointing into THIS bin/ are
    # reclaimed (touch only symlinks resolving into <prefix>/bin, never dirs).
    install(CODE "
        set(_ln [==[${_link_names}]==])
        foreach(_n IN LISTS _ln)
            set(_l \"\${CMAKE_INSTALL_PREFIX}/\${_n}\")
            if(EXISTS \"\${_l}\" OR IS_SYMLINK \"\${_l}\")
                file(REMOVE \"\${_l}\")
            endif()
            file(CREATE_LINK \"bin/${_sname}\" \"\${_l}\" SYMBOLIC RESULT _r)
            if(NOT _r EQUAL 0)
                file(INSTALL \"\${CMAKE_INSTALL_PREFIX}/bin/${_sname}\"
                     DESTINATION \"\${CMAKE_INSTALL_PREFIX}\" RENAME \"\${_n}\"
                     PERMISSIONS OWNER_READ OWNER_WRITE GROUP_READ WORLD_READ
                                 OWNER_EXECUTE GROUP_EXECUTE WORLD_EXECUTE)
            endif()
        endforeach()
        file(GLOB _rootLinks \"\${CMAKE_INSTALL_PREFIX}/*-server\")
        foreach(_l IN LISTS _rootLinks)
            if(NOT IS_SYMLINK \"\${_l}\")
                continue()
            endif()
            get_filename_component(_n \"\${_l}\" NAME)
            list(FIND _ln \"\${_n}\" _ix)
            if(NOT _ix EQUAL -1)
                continue()
            endif()
            # python predicate (prefix / readlink()).resolve().parent == bin_dir —
            # join the RAW link target first (REALPATH on a dangling link returns
            # the link path itself — measured — dead old-brand links would linger).
            file(READ_SYMLINK \"\${_l}\" _raw)
            if(IS_ABSOLUTE \"\${_raw}\")
                set(_joined \"\${_raw}\")
            else()
                get_filename_component(_joined \"\${CMAKE_INSTALL_PREFIX}/\${_raw}\" REALPATH)
            endif()
            get_filename_component(_dir \"\${_joined}\" DIRECTORY)
            get_filename_component(_bin \"\${CMAKE_INSTALL_PREFIX}/bin\" REALPATH)
            if(_dir STREQUAL _bin)
                message(STATUS \"  cleaning old-brand entry link: \${_n}\")
                file(REMOVE \"\${_l}\")
            endif()
        endforeach()
    ")
endfunction()

# -- host tree: lands with the SR7 T3 stroke (the 8 branded names + reclaim + link) --
function(_mediaservo_app_tree_host)
    message(FATAL_ERROR "MEDIASERVO_APP_TREE=host lands with the SR7 T3 stroke")
endfunction()
