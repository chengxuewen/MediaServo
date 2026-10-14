/* imgui_shell — the SDL3 + Dear ImGui rendering shell (the W0 skeleton of p3-gui-viewer W3).
 *
 * W0 surface: window lifecycle + the ImGui context + frame-loop beats (event pump / frame begin / frame end).
 * W3 surface (reserved in this header): the texture pipeline (I420 → SDL texture upload), touch TouchPadding,
 * IME/soft keyboard. (The responsive layout has been superseded by docking's native panes — see ShellOptions::docking.)
 *
 * Dependency archives and version anchors = 3rdparty/PROVENANCE-gui-deps.md (SDL release-3.4.16 /
 * imgui v1.92.9b-docking commit-pin, the IMGUI_HAS_DOCK fingerprint). Backends = imgui_impl_sdl3 + imgui_impl_sdlrenderer3
 * (the SDL_Renderer path; the GPU texture surface will be judged against sdlgpu3 in W3).
 */
#ifndef MEDIASERVO_IMAGUI_SHELL_HPP
#define MEDIASERVO_IMAGUI_SHELL_HPP

#include <string>

namespace imgui_shell {

struct WindowSpec {
    std::string title = "MediaServo";
    /// <=0 = adaptive: take 80% of the primary display's work area and center it (dummy / display-less environments fall back to 1280x720).
    /// An explicit positive value = a fixed initial size (SDL3 windows are draggable-resizable by default; the initial value is only a starting point).
    int width = 0;
    int height = 0;
};

/// Shell behavior options (the default form = byte-for-byte identical for all existing consumers: control_demo's source compatibility is the regression reference).
struct ShellOptions {
    /// true = enable ImGuiConfigFlags_DockingEnable (drag-to-dock windows + a usable DockSpace).
    /// Default off: globalizing the switch would put drag borders on windows with no docking layout (a visual regression),
    /// so each app opts in explicitly — viewer on, control_demo off (the control-demo identity is preserved by this parameter).
    bool docking = false;

    /// Layout persistence file (empty = writing to disk disabled). imgui's default is to write <exe>.imgui.ini into the CWD —
    /// that pollutes build/CI directories (live proof 09-23: an imgui.ini at the submodule root was once tracked), hence explicit opt-in.
    /// Writing is unconditionally disabled under the dummy video driver (after a headless-judgment run, git status must show zero dirt).
    std::string ini_path;
};

/// The rendering shell application (non-copyable; single-threaded use — the GUI main-thread-owner model).
class App {
public:
    explicit App(const WindowSpec& spec, const ShellOptions& opt = {});
    ~App();

    App(const App&) = delete;
    App& operator=(const App&) = delete;

    /// false after a quit event (window close/quit) — the frame-loop sentinel.
    bool running() const;

    /// Drain the SDL event queue (non-blocking). Returning false = the user requested exit (same semantics as running).
    bool pump_events();

    /// ImGui frame begin (the caller guarantees events are pumped before NewFrame).
    void begin_frame();

    /// ImGui frame end + renderer submit + window present.
    void end_frame();

    /// The underlying SDL_Window (used by the W3 texture pipeline; the shell owns it).
    void* sdl_window() const;

    /// Main-thread SDL_Renderer* (for the video texture station; nullptr when no window was created).
    void* sdl_renderer() const;

    /// Whether the ini_path file already existed at construction (= a saved layout from last time). Used by the dock first-run layout guard:
    /// with a saved layout → do not run DockBuilder (otherwise every launch would overwrite the user's drags — the #1 docking newcomer pitfall).
    /// Always false under dummy / write-disabled scenarios (the first-run form; consistent with dock_layout's reset path).
    bool had_saved_layout() const;

private:
    struct Impl;
    Impl* impl_;
};

} // namespace imgui_shell

#endif /* MEDIASERVO_IMAGUI_SHELL_HPP */
