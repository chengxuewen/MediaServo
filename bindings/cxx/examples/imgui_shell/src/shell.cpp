#include "imgui_shell/shell.hpp"

#include <SDL3/SDL.h>
#include <backends/imgui_impl_sdl3.h>
#include <backends/imgui_impl_sdlrenderer3.h>
#include <imgui.h>

namespace imgui_shell {

struct App::Impl {
    SDL_Window* window = nullptr;
    SDL_Renderer* renderer = nullptr;
    bool running = false;
    bool backends_ready = false;  // imgui context/后端 init 全成才置位（析构门）
};

App::App(const WindowSpec& spec) : impl_(new Impl) {
    if (!SDL_Init(SDL_INIT_VIDEO)) {
        SDL_Log("imgui_shell: SDL_Init(video) failed: %s", SDL_GetError());
        return; // running()=false → 主循环零圈直接收摊（无显示环境=CI 编译面可过）
    }
    impl_->window = SDL_CreateWindow(spec.title.c_str(), spec.width, spec.height, 0);
    if (!impl_->window) {
        SDL_Log("imgui_shell: CreateWindow failed: %s", SDL_GetError());
        return;
    }
    impl_->renderer = SDL_CreateRenderer(impl_->window, nullptr); // 官方默认驱动选择
    if (!impl_->renderer) {
        SDL_Log("imgui_shell: CreateRenderer failed: %s", SDL_GetError());
        return;
    }
    ImGui::CreateContext();
    ImGui::StyleColorsDark();
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

App::~App() {
    if (impl_->backends_ready) {
        ImGui_ImplSDLRenderer3_Shutdown();
        ImGui_ImplSDL3_Shutdown();
        ImGui::DestroyContext();
    }
    if (impl_->renderer) SDL_DestroyRenderer(impl_->renderer);
    if (impl_->window) SDL_DestroyWindow(impl_->window);
    SDL_Quit(); // 幂等（SDL3）；init 失败路径亦安全
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
    SDL_SetRenderDrawColor(impl_->renderer, 0x10, 0x10, 0x14, 0xFF); // 黑底（§7 视频主区底色）
    SDL_RenderClear(impl_->renderer);
    ImGui_ImplSDLRenderer3_RenderDrawData(ImGui::GetDrawData(), impl_->renderer);
    SDL_RenderPresent(impl_->renderer);
}

void* App::sdl_window() const { return impl_->window; }

} // namespace imgui_shell
