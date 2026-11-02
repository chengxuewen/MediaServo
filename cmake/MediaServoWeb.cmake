# MediaServoWeb.cmake — www node lanes (S5; F21-B in-scope, unblocked by supplier D43/SR1)
#
# Shape:
#   * polyorch_node_setup: the packageManager field in www/package.json
#     (pnpm@10.32.1) selects the pnpm dispatch — SR1 made the supplier read it;
#     no mirror of the member list into package.json (the F22-a rejection holds).
#   * polyorch_node_import: enumerates apps/* + packages/* from
#     pnpm-workspace.yaml and registers a `-build` mediator per member; the
#     `workspace:*` dependency in @mediaservo/admin on @mediaservo/client
#     becomes a REAL add_dependencies edge — this is the PIT-194 class fix:
#     touching packages/client and building admin rebuilds client first,
#     enforced by the graph, not by mtime luck or human memory.
#   * mediaservo-web-build: aggregate button over every imported member.
#   * The CLI `build web` verb intentionally KEEPS python's mtime-incremental
#     `_ensure_admin_dist` path (proven, incremental-fast for staging flows);
#     the lanes are the IDE/CI surface riding the same tree (§7.11). S5 audit
#     compares one full lane build against the python-produced dist (content
#     equivalence, not byte identity of minified maps).
#   * Configure without a node toolchain: node_setup degrades to FOUND=FALSE
#     and import skips with its STATUS line — a fresh box still configures the
#     web preset green (loud skip), matching the supplier degrade doctrine.

function(mediaservo_setup_web)
    if(NOT MEDIASERVO_BUILD_WEB)
        return()
    endif()
    # Point the supplier's packageManager probe at OUR root manifest (the
    # default probes CMAKE_SOURCE_DIR/package.json — the repo's tooling file,
    # not the workspace root). Caller-scope variable, read by the callee.
    if(NOT PolyOrchNodePackageJson)
        set(PolyOrchNodePackageJson "${MEDIASERVO_REPO_ROOT}/www/package.json")
    endif()
    polyorch_node_setup()
    if(NOT POLYORCH_NODE_FOUND)
        message(STATUS "MediaServo web lanes: node toolchain absent — lanes SKIPPED "
                       "(install node or run via an activated shell; CLI `build web` "
                       "path is unaffected)")
        return()
    endif()
    # namespace the npm handles. Prefix cannot be empty: the crates lanes mount
    # mediaservo-<name>-build in server/all, so a bare handle would collide with
    # same-named crates (supplier duplicate-handle FATAL); "mediaservo-web" keeps
    # every owned target sorted under mediaservo-*. STRIP_SCOPES then removes the
    # scope word so @mediaservo/admin does not double it -- today's forms
    # 'mediaservo-web-mediaservo-admin-build' / 'mediaservo-web-mediaservo-client-build'
    # carry "mediaservo" twice at zero information and read near-colliding against
    # the crate-lane targets mediaservo-client / mediaservo-admin. With the strip
    # (supplier D47, PolyOrch >= 0.7.2): 'mediaservo-web-admin-build' /
    # 'mediaservo-web-client-build'.
    if(NOT PolyOrch_NODE_TARGET_PREFIX)
        set(PolyOrch_NODE_TARGET_PREFIX "mediaservo-web")
    endif()
    if(NOT PolyOrch_NODE_STRIP_SCOPES)
        set(PolyOrch_NODE_STRIP_SCOPES "mediaservo")
    endif()
    # Family IDE root (plan ide-folder-hierarchy, ruling 3): the node helpers join
    # every creation site to this knob "only when set" (PolyOrchNodeHelpers:522-525,
    # default NONE per :68-72) — one set carries the whole web family into the
    # mediaservo/web group (standalone view: bare "web"). Function-scoped plain
    # set: does not leak to other faces (rule 6 posture, mirrored at :50-55).
    set(PolyOrch_RUST_FOLDER_ROOT "${MEDIASERVO_IDE_ROOT}web")
    polyorch_node_import(ROOT "${MEDIASERVO_REPO_ROOT}/www/package.json")
    set(_builds "")
    foreach(_h IN LISTS POLYORCH_NODE_IMPORT_HANDLES)
        if(TARGET "${_h}-build")
            list(APPEND _builds "${_h}-build")
        endif()
    endforeach()
    if(_builds)
        add_custom_target(mediaservo-web-build
            DEPENDS ${_builds}
            COMMENT "node: build all www members (admin chain rides the client edge)")
        # The aggregate is a plain add_custom_target — the knob only carries the
        # mediator family; the button joins its members explicitly (same value).
        set_target_properties(mediaservo-web-build PROPERTIES
            FOLDER "${MEDIASERVO_IDE_ROOT}web")
        message(STATUS "MediaServo web lanes: mediaservo-web-build => ${_builds}")
    endif()
endfunction()
