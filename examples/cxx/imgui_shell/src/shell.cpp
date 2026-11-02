#include "imgui_shell/shell.hpp"

#include <SDL3/SDL.h>
#include <cstring>
#include <filesystem>  // exists (layout-archive probe)  // std::strcmp (dummy check)
#include <backends/imgui_impl_sdl3.h>
#include <backends/imgui_impl_sdlrenderer3.h>
#include <imgui.h>

#ifdef MSRTC_X11
#include <X11/Xlib.h>  // X11 form only: install a non-exiting error handler (see the CreateWindow note)
namespace {
int x11_tolerant_error_handler(Display*, XErrorEvent* ev) {
    // The default Xlib handler exit(1)s on any X error. SDL 3.4.16 has no global handler, and the None atoms
    // negotiated by Xwayland selection/property trigger BadAtom and kill the process outright (recorded in the 09-20 window round;
    // see patches/sdl3-x11-none-guard.py). This handler logs and returns 0 = the failing call returns
    // NULL and SDL continues based on the return value; works together with the events.c None/NULL guards (missing either means a second-order crash).
    // Dedup: log an error_code once on first occurrence + one summary line per 100 occurrences (the Xwayland clipboard negotiation storm is a benign constant,
    // but a flood makes "examples as templates" look broken — a live 09-24 user run recorded 20+ lines/minute)
    static int seen[32] = {0};
    const int slot = ev->error_code & 31;
    if (++seen[slot] == 1 || seen[slot] % 100 == 0)
        SDL_Log("imgui_shell: ignoring non-fatal X11 error code=%d req=%d (occurrence %d; routine Xwayland negotiation)",
                ev->error_code, ev->request_code, seen[slot]);
    return 0;
}
}  // namespace
#endif

namespace imgui_shell {

struct App::Impl {
    SDL_Window* window = nullptr;
    SDL_Renderer* renderer = nullptr;
    bool running = false;
    bool backends_ready = false;  // set only when the imgui context/backends all init (the destructor gate)
    std::string ini_name;         // io.IniFilename points at it — the lifetime must follow Impl (imgui stores only a raw pointer)
    bool had_saved_layout = false;  // the construction-time probe result (forwarded by App::had_saved_layout)
};

App::App(const WindowSpec& spec, const ShellOptions& opt) : impl_(new Impl) {
    #ifdef MSRTC_X11
    // Must be installed before SDL_Init: SDL already subscribes to clipboard properties during CreateWindow, so an early BadAtom would hit
    // the default Xlib handler (= exit). libX11 is a DT_NEEDED; XSetErrorHandler is process-level and does not depend on the display.
    XSetErrorHandler(x11_tolerant_error_handler);
    #endif
    if (!SDL_Init(SDL_INIT_VIDEO)) {
        SDL_Log("imgui_shell: SDL_Init(video) failed: %s", SDL_GetError());
        return; // running()=false → the main loop runs zero iterations and winds down (no display environment = the CI compile surface still passes)
    }
    // Adaptive initial size: floor at 80% of the primary display (SDL_GetDisplayBounds failure / zero size = the dummy headless form,
    // fall back to a fixed 1280x720 — the headless-judgment path never depends on a display environment).
    int w = spec.width, h = spec.height;
    bool adaptive = (w <= 0 || h <= 0);
    SDL_Rect usable{};
    if (adaptive) {
        // UsableBounds = the work area excluding the GNOME top bar/dock (GetDisplayBounds is the full-screen bounds —
        // computing 80% from it still gets occluded by the dock; proven live by a user on a 1280x768 desktop, 09-24). On failure fall back to bounds,
        // and on failure (dummy headless) fall back to 1280x720.
        if (!SDL_GetDisplayUsableBounds(0, &usable) || usable.w <= 0 || usable.h <= 0) {
            if (!SDL_GetDisplayBounds(0, &usable) || usable.w <= 0 || usable.h <= 0) {
                usable.x = usable.y = 0;
                usable.w = 1280;
                usable.h = 720;
            }
        }
        w = usable.w * 4 / 5;
        h = usable.h * 4 / 5;
    }
    // RESIZABLE: SDL3 windows are not draggable-resizable by default (same as SDL2 — the old "already resizable" note is overturned);
    // without this flag the WM provides no resize/maximize buttons. The adaptive initial size (80% of the primary work area + centered) is handled separately.
    impl_->window = SDL_CreateWindow(spec.title.c_str(), w, h, SDL_WINDOW_RESIZABLE);
    if (adaptive && impl_->window) {
        // Center within the work area (including the origin offset — with a dock on the left, (0,0) is not the visual center; SDL has no Center API in this version)
        SDL_SetWindowPosition(impl_->window, usable.x + (usable.w - w) / 2,
                               usable.y + (usable.h - h) / 2);
    }
    if (!impl_->window) {
        SDL_Log("imgui_shell: CreateWindow failed: %s", SDL_GetError());
        return;
    }
    impl_->renderer = SDL_CreateRenderer(impl_->window, nullptr); // the official default driver selection
    if (!impl_->renderer) {
        SDL_Log("imgui_shell: CreateRenderer failed: %s", SDL_GetError());
        return;
    }
    ImGui::CreateContext();
    ImGui::StyleColorsDark();

    // -- docking and layout-persistence discipline (semantics explained in the shell.hpp ShellOptions comments) --
    ImGuiIO& io = ImGui::GetIO();
    if (opt.docking) io.ConfigFlags |= ImGuiConfigFlags_DockingEnable;
    const char* drv = SDL_GetCurrentVideoDriver();
    const bool headless = !drv || std::strcmp(drv, "dummy") == 0;
    if (!opt.ini_path.empty() && !headless) {
        std::error_code ec;  // the non-throwing exists — a read-only probe; success or failure never blocks startup
        impl_->had_saved_layout = std::filesystem::exists(opt.ini_path, ec);
        impl_->ini_name = opt.ini_path;            // copy into Impl; io holds the c_str()
        io.IniFilename = impl_->ini_name.c_str();
    } else {
        io.IniFilename = nullptr;  // writing disabled: prevents CWD pollution (live proof 09-23 = an imgui.ini at the submodule root was once tracked)
    }
    if (!ImGui_ImplSDL3_InitForSDLRenderer(impl_->window, impl_->renderer) ||
        !ImGui_ImplSDLRenderer3_Init(impl_->renderer)) {
        SDL_Log("imgui_shell: imgui backend init failed");
        SDL_DestroyRenderer(impl_->renderer);
        impl_->renderer = nullptr;
        return;
    }
    impl_->backends_ready = true;
    impl_->running = true;
}

bool App::had_saved_layout() const { return impl_->had_saved_layout; }

App::~App() {
    if (impl_->backends_ready) {
        ImGui_ImplSDLRenderer3_Shutdown();
        ImGui_ImplSDL3_Shutdown();
        ImGui::DestroyContext();
    }
    if (impl_->renderer) SDL_DestroyRenderer(impl_->renderer);
    if (impl_->window) SDL_DestroyWindow(impl_->window);
    SDL_Quit(); // idempotent (SDL3); also safe on the init-failure path
    delete impl_;
}

bool App::running() const { return impl_->running; }

bool App::pump_events() {
    SDL_Event ev;
    while (SDL_PollEvent(&ev)) {
        ImGui_ImplSDL3_ProcessEvent(&ev);
        if (ev.type == SDL_EVENT_QUIT) {
            impl_->running = false;
        }
    }
    return impl_->running;
}

void App::begin_frame() {
    ImGui_ImplSDLRenderer3_NewFrame();
    ImGui_ImplSDL3_NewFrame();
    ImGui::NewFrame();
}

void App::end_frame() {
    ImGui::Render();
    SDL_SetRenderDrawColor(impl_->renderer, 0x10, 0x10, 0x14, 0xFF); // black base (§7's video main-area background)
    SDL_RenderClear(impl_->renderer);
    ImGui_ImplSDLRenderer3_RenderDrawData(ImGui::GetDrawData(), impl_->renderer);
    SDL_RenderPresent(impl_->renderer);
}

void* App::sdl_window() const { return impl_->window; }

void* App::sdl_renderer() const { return impl_->renderer; }

} // namespace imgui_shell
