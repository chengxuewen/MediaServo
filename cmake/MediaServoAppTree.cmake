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
#   * Build assembly mode (SR6 batch-5): `build server`/`build web` ride THIS face
#     through MEDIASERVO_APP_STAGING ('' = deploy — every existing deploy byte
#     parity untouched | build | web). build: the bin/weaknet rows source the FRESH
#     target/{profile} artifacts first (a previous build's out/bin copy must never
#     mask a just-compiled binary — PIT-189 family) through swap legs — file(COPY)
#     into a side dir + file(RENAME) onto the landing name, the atomic .new/
#     os.replace semantics python's staging had: install(PROGRAMS) truncates the
#     destination and would ETXTBSY a LIVE cluster's running bin, and deploy may
#     truncate only because it stops the cluster first — build must never stop
#     anything. The web tree moves dist -> <prefix>/web (rmtree+copytree parity).
#     Root entry shortcuts are a deploy landing: NEVER registered in build mode
#     (the delivery tree stays bin/etc/web exactly like python laid it).

set(MEDIASERVO_APP_TREE "" CACHE STRING "app delivery layout to describe at install time: '' (none) | host | server")
set(MEDIASERVO_BRAND "" CACHE STRING "product brand for the physical bin names (D269); empty = upstream names byte-stable")
set(MEDIASERVO_SERVER_OUT_DIR "" CACHE PATH "server delivery root (the CLI-resolved <out>/server) — server placement rows source from here")
set(MEDIASERVO_APP_STAGING "" CACHE STRING "server-tree placement mode: '' (deploy: delivery-tree-first PROGRAMS route) | build (fresh target/{profile} bins via swap legs + web move) | web (web placement only — the build web fast lane)")

# The eight host-tree bin names as cargo spells them (the CLI's HOST_BINS mirror —
# D269 names: host-<app> -> <brand>-<app>, mediaservo-host -> <brand>-host).
set(MEDIASERVO_HOST_BINS mediaservo-host host-agent host-streamer host-capturer
    host-recorder host-controller host-emergency host-audio)

function(mediaservo_setup_app_tree)
    # The web fast lane rides the same gate: staging=web registers ONLY the web
    # placement legs — build web never touches cargo, so the bin hunt's FATAL
    # polarity must not fire (the tree value stays 'server': the web tree IS the
    # server delivery tree).
    if(MEDIASERVO_APP_STAGING STREQUAL "web")
        _mediaservo_app_web_legs()
        return()
    endif()
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

# -- web placement legs (SR6 batch-5, absorbed from python _stage_web_to_out):
#    the vite dist tree moves wholesale — rmtree+copytree parity = a rm -rf leg
#    registered BEFORE the directory copy (install events run in registration
#    order; P1-pack measured fact). Missing dist keeps the python exit-2 error
#    verbatim at configure time (loud, never a silent hollow web/).
function(_mediaservo_app_web_legs)
    set(_dist "${MEDIASERVO_REPO_ROOT}/www/apps/admin/dist")
    if(NOT EXISTS "${_dist}/index.html")
        message(FATAL_ERROR
            "Error: no artifacts in ${_dist} — pnpm build:admin not run or failed")
    endif()
    install(CODE "execute_process(COMMAND \"\${CMAKE_COMMAND}\" -E rm -rf \"\${CMAKE_INSTALL_PREFIX}/web\")")
    install(DIRECTORY "${_dist}/" DESTINATION web USE_SOURCE_PERMISSIONS)
endfunction()

# -- swap-install leg (build mode only): file(COPY) into a side dir (preserves
#    modes like python's copy2) then file(RENAME) onto the landing name — the
#    atomic replace of the retired shutil.copy2 + os.replace pattern. The
#    transient dir self-cleans long before the reclaim leg globs bin/.
#    ponytail: RENAME-over-existing untested on Windows (server builds are the
#    mediasoup-linux domain; the Windows faces stay out of scope).
function(_mediaservo_app_swap_leg src land_name)
    install(CODE "
        file(MAKE_DIRECTORY \"\${CMAKE_INSTALL_PREFIX}/bin\")
        set(_sw \"\${CMAKE_INSTALL_PREFIX}/bin/.swap-${land_name}\")
        execute_process(COMMAND \"\${CMAKE_COMMAND}\" -E rm -rf \"\${_sw}\")
        file(MAKE_DIRECTORY \"\${_sw}\")
        file(COPY [==[${src}]==] DESTINATION \"\${_sw}\")
        get_filename_component(_bn [==[${src}]==] NAME)
        file(RENAME \"\${_sw}/\${_bn}\" \"\${CMAKE_INSTALL_PREFIX}/bin/${land_name}\")
        execute_process(COMMAND \"\${CMAKE_COMMAND}\" -E rm -rf \"\${_sw}\")
        message(STATUS \"  bin/${land_name}: staged via swap install (build assembly)\")
    ")
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
    # cargo profile for build-mode sourcing (the host tree's same idiom).
    string(TOLOWER "${CMAKE_BUILD_TYPE}" _profile)
    if(NOT _profile)
        set(_profile debug)
    endif()

    # BLOCKER-1 dual-source hunt (ordered): upstream name (fresh build) -> brand
    # name (already-migrated tree: a second deploy needs no rebuild). Missing in
    # out/ = deploy precondition failure (D266: deploy does not trigger builds) —
    # same message the CLI used to print before its copy loop. Build assembly
    # prepends the just-compiled target/{profile} artifact so it always wins over
    # any previous build's out/bin residue.
    set(_candidates "")
    if(MEDIASERVO_APP_STAGING STREQUAL "build")
        list(APPEND _candidates "${MEDIASERVO_REPO_ROOT}/target/${_profile}/mediaservo-server${_exe}")
    endif()
    list(APPEND _candidates "${MEDIASERVO_SERVER_OUT_DIR}/bin/mediaservo-server${_exe}")
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
    # Build mode sources from target/ (never a same-name truncation of a live
    # out/bin binary) and lands through the swap leg instead.
    if(MEDIASERVO_APP_STAGING STREQUAL "build")
        _mediaservo_app_swap_leg("${_src}" "${_sname}")
    else()
        install(PROGRAMS "${_src}" DESTINATION bin RENAME "${_sname}")
    endif()

    # weaknet dual source + target/ fallback + absent = WARN skip ([bF4 bF5] —
    # never blocks the server main flow; the oxfile conditional-render predicate
    # belongs to Rust init, nailed by M-1 — cmake only settles the data plane).
    # Build mode puts the freshly compiled target/{profile} binary first.
    set(_wcands "")
    if(MEDIASERVO_APP_STAGING STREQUAL "build")
        list(APPEND _wcands "${MEDIASERVO_REPO_ROOT}/target/${_profile}/mediaservo-weaknet${_exe}")
    endif()
    list(APPEND _wcands "${MEDIASERVO_SERVER_OUT_DIR}/bin/mediaservo-weaknet${_exe}")
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
        if(MEDIASERVO_APP_STAGING STREQUAL "build")
            _mediaservo_app_swap_leg("${_wsrc}" "${_wname}")
        else()
            install(PROGRAMS "${_wsrc}" DESTINATION bin RENAME "${_wname}")
        endif()
    else()
        message(WARNING
            "weaknet binary absent from all locations (out/server/bin and target/) — "
            "bin/${_wname} is NOT staged this time (does not block deploy; run build server)")
    endif()

    # etc mirror (SR7 T2): the delivery tree's staged configs (server.yaml + the
    # accounts/devices templates — whatever build staged) land into <prefix>/etc
    # FIRST-WRITER-WINS: the PIT-160 contract (ops runtime data — hot-written
    # devices.yaml, PSK-edited server.yaml — survives redeploys; proven as-found,
    # the live out/etc bytes differ from config/ templates, so the source is the
    # delivery tree, never the templates). install(FILES) rows cannot express a
    # destination-exists condition, so this is one script-mode CODE leg over a
    # configure-time glob (freshness rides the always-reconfigure at the call
    # site); file(COPY) preserves modes exactly like python copy2 (664 family).
    file(GLOB _etcFiles LIST_DIRECTORIES FALSE "${MEDIASERVO_SERVER_OUT_DIR}/etc/*")
    set(_etcKeep "")
    foreach(_f IN LISTS _etcFiles)
        if(EXISTS "${_f}" AND NOT IS_DIRECTORY "${_f}")
            list(APPEND _etcKeep "${_f}")
        endif()
    endforeach()
    list(SORT _etcKeep)
    if(_etcKeep)
        install(CODE "
            file(MAKE_DIRECTORY \"\${CMAKE_INSTALL_PREFIX}/etc\")
            set(_etcSrc [==[${_etcKeep}]==])
            foreach(_f IN LISTS _etcSrc)
                get_filename_component(_n \"\${_f}\" NAME)
                if(EXISTS \"\${CMAKE_INSTALL_PREFIX}/etc/\${_n}\")
                    continue()
                endif()
                file(COPY \"\${_f}\" DESTINATION \"\${CMAKE_INSTALL_PREFIX}/etc\")
                message(STATUS \"  etc/\${_n}: template landed\")
            endforeach()
        ")
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
    # Build assembly NEVER registers these legs: the delivery tree stays
    # bin/etc/web exactly like python's build staged it (deploy adds links).
    if(NOT MEDIASERVO_APP_STAGING STREQUAL "build")
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
    endif()

    # build assembly: the web move rides the SAME --install pass (registered after
    # the bin legs; python staged web first — one pass, end state identical).
    if(MEDIASERVO_APP_STAGING STREQUAL "build")
        _mediaservo_app_web_legs()
    endif()
endfunction()

# -- host tree: the 8 branded app bins + reclaim + entry link (SR7 T3) --
# Sources are the cargo workspace target/<profile> (build host is pure compile —
# D266; this is where _assemble_host_binaries read from). Per-name N3 rule: every
# name is cmake-visible (target-dir artifact) -> row, zero ruled-away bins.
# oxmgr (vendor archive layer, plan §5) and env.sh (generated text) STAY python —
# ruled-away markers live at their call sites in mediaservo_cli.py.
function(_mediaservo_app_tree_host)
    set(_exe "${CMAKE_EXECUTABLE_SUFFIX}")
    string(TOLOWER "${CMAKE_BUILD_TYPE}" _profile)
    if(NOT _profile)
        set(_profile debug)
    endif()
    set(_srcdir "${MEDIASERVO_REPO_ROOT}/target/${_profile}")
    set(_landed "")
    set(_hostdn "")
    foreach(_b IN LISTS MEDIASERVO_HOST_BINS)
        set(_art "${_srcdir}/${_b}${_exe}")
        # missing-src skip = python's "continue" for unbuilt bins (a partial
        # build deploys the names that exist; keep manifest tracks landings).
        if(NOT EXISTS "${_art}")
            continue()
        endif()
        if(MEDIASERVO_BRAND)
            if(_b STREQUAL "mediaservo-host")
                set(_dn "${MEDIASERVO_BRAND}-host${_exe}")
            else()
                string(SUBSTRING "${_b}" 5 -1 _app)   # strip "host-"
                set(_dn "${MEDIASERVO_BRAND}-${_app}${_exe}")
            endif()
        else()
            set(_dn "${_b}${_exe}")
        endif()
        install(PROGRAMS "${_art}" DESTINATION bin RENAME "${_dn}")
        list(APPEND _landed "${_dn}")
        if(_b STREQUAL "mediaservo-host")
            set(_hostdn "${_dn}")
        endif()
    endforeach()
    if(NOT _landed)
        message(FATAL_ERROR
            "no host binaries found under ${_srcdir} — run build host first "
            "(deploy does not trigger builds — D266)")
    endif()

    # bin whitelist reclaim (deploy-ops 4 + the client/streamer-compat-chain
    # residue of D269): delete role-suffixed files not landed by THIS layout.
    # Symlinks and directories are never touched (python predicate mirrored).
    # f8f85db rule: list(FIND), not IN_LIST.
    set(_roles agent streamer capturer recorder controller emergency audio legacy host client)
    set(_role_rx "")
    foreach(_r IN LISTS _roles)
        string(APPEND _role_rx "|-${_r}${_exe}")
    endforeach()
    string(SUBSTRING "${_role_rx}" 1 -1 _role_rx)
    install(CODE "
        set(_keep [==[${_landed}]==])
        file(GLOB _binFiles LIST_DIRECTORIES FALSE \"\${CMAKE_INSTALL_PREFIX}/bin/*\")
        foreach(_f IN LISTS _binFiles)
            if(IS_SYMLINK \"\${_f}\" OR IS_DIRECTORY \"\${_f}\")
                continue()
            endif()
            get_filename_component(_n \"\${_f}\" NAME)
            list(FIND _keep \"\${_n}\" _ix)
            if(NOT _ix EQUAL -1)
                continue()
            endif()
            # suffix match (python endswith on dash-role) -- no leading anchor;
            # each alternative already starts with the dash.
            if(_n MATCHES \"(${_role_rx})\$\")
                message(STATUS \"  cleaning deployment residue: \${_n}\")
                file(REMOVE \"\${_f}\")
            endif()
        endforeach()
    ")

    # root entry shortcut (MED4 shape): relative symlink to the real branded CLI
    # (empty brand = the dual upstream shortcuts, never a "-host" leftover).
    # The dangling-link case mirrors python (symlink_to succeeds anyway); the
    # copy fallback only runs where symlinks are unsupported.
    if(MEDIASERVO_BRAND)
        set(_link_names "${MEDIASERVO_BRAND}-host")
    else()
        set(_link_names "host;mediaservo-host")
    endif()
    install(CODE "
        set(_ln [==[${_link_names}]==])
        foreach(_n IN LISTS _ln)
            set(_l \"\${CMAKE_INSTALL_PREFIX}/\${_n}\")
            if(EXISTS \"\${_l}\" OR IS_SYMLINK \"\${_l}\")
                file(REMOVE \"\${_l}\")
            endif()
            file(CREATE_LINK \"bin/${_hostdn}\" \"\${_l}\" SYMBOLIC RESULT _r)
            if(NOT _r EQUAL 0)
                file(INSTALL \"\${CMAKE_INSTALL_PREFIX}/bin/${_hostdn}\"
                     DESTINATION \"\${CMAKE_INSTALL_PREFIX}\" RENAME \"\${_n}\"
                     PERMISSIONS OWNER_READ OWNER_WRITE GROUP_READ WORLD_READ
                                 OWNER_EXECUTE GROUP_EXECUTE WORLD_EXECUTE)
            endif()
        endforeach()
    ")
endfunction()
