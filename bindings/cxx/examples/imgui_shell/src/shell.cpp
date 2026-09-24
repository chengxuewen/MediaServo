#include "imgui_shell/shell.hpp"

#include <SDL3/SDL.h>
#include <cstring>
#include <filesystem>  // exists（布局存档探测）  // std::strcmp（dummy 判）
#include <backends/imgui_impl_sdl3.h>
#include <backends/imgui_impl_sdlrenderer3.h>
#include <imgui.h>

#ifdef MSRTC_X11
#include <X11/Xlib.h>  // 仅 X11 形态：安装非 exit 的 error handler（见 CreateWindow 处注）
namespace {
int x11_tolerant_error_handler(Display*, XErrorEvent* ev) {
    // Xlib 默认 handler 对任何 X error exit(1)。SDL 3.4.16 无全局 handler，Xwayland
    // selection/property 协商的 None 原子触发 BadAtom 会直接杀进程（09-20 出窗轮实录，
    // 见 patches/sdl3-x11-none-guard.py）。此 handler 记日志并 return 0 = 让失败调用返回
    // NULL 后 SDL 按返回值处理继续；与 events.c 的 None/NULL 守护配合（缺一即二阶崩）。
    // 去重：同 error_code 只首播 + 每 100 次汇总一条（Xwayland 剪贴板协商风暴是良性常态，
    // 刷屏会让"示例即模板"看起来像坏了——09-24 用户实跑 20+ 条/分钟实录）
    static int seen[32] = {0};
    const int slot = ev->error_code & 31;
    if (++seen[slot] == 1 || seen[slot] % 100 == 0)
        SDL_Log("imgui_shell: 忽略非致命 X11 error code=%d req=%d（第 %d 次，Xwayland 协商常态）",
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
    bool backends_ready = false;  // imgui context/后端 init 全成才置位（析构门）
    std::string ini_name;         // io.IniFilename 指它——生命周期必须随 Impl（imgui 只存裸指针）
    bool had_saved_layout = false;  // 构造期探测结果（App::had_saved_layout 转发）
};

App::App(const WindowSpec& spec, const ShellOptions& opt) : impl_(new Impl) {
    #ifdef MSRTC_X11
    // 须在 SDL_Init 前装：CreateWindow 期间 SDL 已订阅 clipboard 属性，早期 BadAtom 会撞
    // Xlib 默认 handler(=exit)。libX11 为 DT_NEEDED，XSetErrorHandler 进程级、不依赖 display。
    XSetErrorHandler(x11_tolerant_error_handler);
    #endif
    if (!SDL_Init(SDL_INIT_VIDEO)) {
        SDL_Log("imgui_shell: SDL_Init(video) failed: %s", SDL_GetError());
        return; // running()=false → 主循环零圈直接收摊（无显示环境=CI 编译面可过）
    }
    // 自适应初始尺寸：主显示 80% 取整（SDL_GetDisplayBounds 失败/零尺寸=dummy 无头形，
    // 回退固定 1280x720——无头判据路径零依赖显示环境）。
    int w = spec.width, h = spec.height;
    bool adaptive = (w <= 0 || h <= 0);
    SDL_Rect usable{};
    if (adaptive) {
        // UsableBounds=扣掉 GNOME 顶栏/dock 的工作区（GetDisplayBounds 是全屏边界——
        // 拿它算 80% 仍会被 dock 遮，09-24 用户实锤 1280x768 桌面）。失败回退 bounds，
        // 再失败（dummy 无头）回退 1280x720。
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
    impl_->window = SDL_CreateWindow(spec.title.c_str(), w, h, 0);
    if (adaptive && impl_->window) {
        // 居中于工作区（含原点偏移——dock 在左侧时 (0,0) 不是视觉中心；本档期 SDL 无 Center API）
        SDL_SetWindowPosition(impl_->window, usable.x + (usable.w - w) / 2,
                               usable.y + (usable.h - h) / 2);
    }
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

    // —— docking 与布局持久化纪律（语义解释在 shell.hpp ShellOptions 注释）——
    ImGuiIO& io = ImGui::GetIO();
    if (opt.docking) io.ConfigFlags |= ImGuiConfigFlags_DockingEnable;
    const char* drv = SDL_GetCurrentVideoDriver();
    const bool headless = !drv || std::strcmp(drv, "dummy") == 0;
    if (!opt.ini_path.empty() && !headless) {
        std::error_code ec;  // 非抛出 exists——只读探测，成败都不拦启动
        impl_->had_saved_layout = std::filesystem::exists(opt.ini_path, ec);
        impl_->ini_name = opt.ini_path;            // 拷贝归 Impl，io 持 c_str()
        io.IniFilename = impl_->ini_name.c_str();
    } else {
        io.IniFilename = nullptr;  // 禁写盘：防 CWD 污染（09-23 活证=子仓根 imgui.ini 曾被跟踪）
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

void* App::sdl_renderer() const { return impl_->renderer; }

} // namespace imgui_shell
