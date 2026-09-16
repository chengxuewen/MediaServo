/* imgui_shell — SDL3 + Dear ImGui 渲染壳（p3-gui-viewer W3 的 W0 骨架）。
 *
 * W0 面：窗口生命周期 + ImGui 上下文 + 帧循环节拍（事件泵/帧首/帧尾）。
 * W3 面（本头预留）：纹理管线（I420→SDL texture 上传）、两档响应式布局
 * （Splitter/抽屉/竖屏）、触摸 TouchPadding、IME/软键盘。
 *
 * 依赖档案与版本锚 = 3rdparty/PROVENANCE-gui-deps.md（SDL release-3.4.16 /
 * imgui v1.92.9b commit-pin）。后端 = imgui_impl_sdl3 + imgui_impl_sdlrenderer3
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

/// 渲染壳应用（非拷贝；单线程使用——GUI 主线程属主模型）。
class App {
public:
    explicit App(const WindowSpec& spec);
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

private:
    struct Impl;
    Impl* impl_;
};

} // namespace imgui_shell

#endif /* MEDIASERVO_IMAGUI_SHELL_HPP */
