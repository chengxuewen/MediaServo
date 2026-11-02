# examples/cxx/crate-examples — C++ usage material (batch ②, plan directory-artifacts-convergence)

Relocated from the four `bindings/cxx/mediaservo-<domain>-cxx/examples/` dirs. This
level is DELIBERATELY invisible to the cxx band G4 glob (`examples/cxx/*/CMakeLists.txt`):
these are e2e/doc-compiled material and a standalone downstream project, not island lanes.

| item | kind | consumer |
|---|---|---|
| `vehicle_field.cpp` | C++ header-only RAII demo (linkage-tested) | `scripts/e2e-bindings.sh` §6 (g++ -std=c++11) |
| `vehicle_link.cpp`  | C++ link demo | docs (`sdk-cxx/link.md` source anchor) |
| `vehicle_deck.cpp`  | C++ deck closed-loop demo | docs (`sdk-cxx/deck.md` source anchor) |
| `ros2_node/`        | standalone ROS2 downstream project (install-state consumption: pkg-config `mediaservo-client`) | built out-of-tree by ROS users |

Compile notes: the deck .so transitively needs the conda libstdc++ rpath-link shape
(same workaround the cxx island carries as a private patch; e2e runs under pixi where
it is native).
