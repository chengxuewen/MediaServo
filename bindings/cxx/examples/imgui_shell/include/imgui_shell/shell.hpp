/* imgui_shell — SDL3 + Dear ImGui 渲染壳（p3-gui-viewer W3 的 W0 骨架）。
 *
 * W0 面：窗口生命周期 + ImGui 上下文 + 帧循环节拍（事件泵/帧首/帧尾）。
 * W3 面（本头预留）：纹理管线（I420→SDL texture 上传）、触摸 TouchPadding、
 * IME/软键盘。（响应式布局已由 docking 原生分栏取代——见 ShellOptions::docking。）
 *
 * 依赖档案与版本锚 = 3rdparty/PROVENANCE-gui-deps.md（SDL release-3.4.16 /
 * imgui v1.92.9b-docking commit-pin，IMGUI_HAS_DOCK 指纹）。后端 = imgui_impl_sdl3 + imgui_impl_sdlrenderer3
 * （SDL_Renderer 通路；GPU 纹理面 W3 再判 sdlgpu3）。
 */
#ifndef MEDIASERVO_IMAGUI_SHELL_HPP
#define MEDIASERVO_IMAGUI_SHELL_HPP

#include <string>

namespace imgui_shell {

struct WindowSpec {
    std::string title = "MediaServo";
    int width = 1280;
    int height = 720;
};

/// 壳行为选项（默认形=既有全消费者逐字节不变：control_demo 源兼容即回归对照）。
struct ShellOptions {
    /// true = 开 ImGuiConfigFlags_DockingEnable（窗口可拖拽停靠 + DockSpace 可用）。
    /// 默认关：开关全局化会让未做 docking 布局的窗口平白出现拖拽边框（视觉回归），
    /// 故按 app 显式 opt-in——viewer 开，control_demo 关（对照物身份靠这个参数保）。
    bool docking = false;

    /// 布局持久化文件（空 = 禁用写盘）。imgui 默认行为是往 CWD 写 <exe>.imgui.ini——
    /// 构建/CI 目录会被污染（09-23 活证：子仓根 imgui.ini 被历史跟踪），故显式 opt-in。
    /// dummy 视频驱动下无条件禁写（无头判据跑完 git status 必须零脏）。
    std::string ini_path;
};

/// 渲染壳应用（非拷贝；单线程使用——GUI 主线程属主模型）。
class App {
public:
    explicit App(const WindowSpec& spec, const ShellOptions& opt = {});
    ~App();

    App(const App&) = delete;
    App& operator=(const App&) = delete;

    /// 收到退出事件（窗口关闭/quit）后为 false——帧循环哨兵。
    bool running() const;

    /// 排空 SDL 事件队列（非阻塞）。返回 false = 用户请求退出（同 running 语义）。
    bool pump_events();

    /// ImGui 帧首（NewFrame 前泵事件由调用方保证）。
    void begin_frame();

    /// ImGui 帧尾 + renderer 提交 + 窗口 present。
    void end_frame();

    /// 底层 SDL_Window（W3 纹理管线用；本壳持有所有权）。
    void* sdl_window() const;

    /// 主线程 SDL_Renderer*（视频纹理工位用；窗口未建 = nullptr）。
    void* sdl_renderer() const;

private:
    struct Impl;
    Impl* impl_;
};

} // namespace imgui_shell

#endif /* MEDIASERVO_IMAGUI_SHELL_HPP */
