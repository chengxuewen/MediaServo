/* imgui_shell 纹理层 — I420 帧 → SDL texture 的跨线程搬运件（W3b）。
 *
 * 线程模型（对齐 SDK frame 回调「仅回调内有效」合同）：
 *   泵线程 → FrameStaging::push()   （latest-only：新帧覆盖未消费的旧帧=天然丢帧不死等）
 *   主线程 → pop() → VideoTexture::update() → texture_id() 给 ImGui::Image
 *
 * 仅 SDL_Renderer 路径（imgui_impl_sdlrenderer3 以 SDL_Texture* 作 ImTextureID）。
 * 头文件刻意不依赖 SDL 类型（void* 句柄），include 顺序自由。
 */
#ifndef MEDIASERVO_IMGUI_SHELL_TEXTURE_HPP
#define MEDIASERVO_IMGUI_SHELL_TEXTURE_HPP

#include <atomic>
#include <cstdint>
#include <mutex>
#include <vector>

namespace imgui_shell {

/// 泵线程→主线程的最新帧槽（一帧拷贝上限，无队列无背压）。
class FrameStaging {
public:
    void push(const void* i420, size_t len, uint32_t w, uint32_t h);

    /// 取走未消费帧（无新帧 = false）。out 复用容量（每帧零稳态分配）。
    bool pop(std::vector<uint8_t>& out, uint32_t* w, uint32_t* h);

    uint64_t pushed() const { return pushed_.load(std::memory_order_relaxed); }

private:
    std::mutex mu_;
    std::vector<uint8_t> slot_;
    uint32_t w_ = 0, h_ = 0;
    bool ready_ = false;
    std::atomic<uint64_t> pushed_{0};
};

/// 主线程视频纹理工位（update 尺寸变化即重建；黑帧不画只报 id=0）。
class VideoTexture {
public:
    VideoTexture() = default;
    ~VideoTexture();
    VideoTexture(const VideoTexture&) = delete;
    VideoTexture& operator=(const VideoTexture&) = delete;

    /// renderer = 主线程 SDL_Renderer*（App::sdl_renderer()）。
    /// i420 = 平面连续 Y(w*h)+U(w*h/4)+V(w*h/4)（SDL IYUV 布局零拷贝直喂）。
    bool update(void* renderer, const void* i420, size_t len, uint32_t w, uint32_t h);

    /// ImGui::Image 用的纹理句柄（SDL_Texture*；未建立 = nullptr）。
    void* texture_id() const { return tex_; }
    uint32_t width() const { return w_; }
    uint32_t height() const { return h_; }
    uint64_t frames() const { return frames_; }

private:
    void destroy();

    void* tex_ = nullptr; // SDL_Texture*（renderer 由主线程 SDL_GetRenderer 现取）
    uint32_t w_ = 0, h_ = 0;
    uint64_t frames_ = 0;
};

} // namespace imgui_shell

#endif /* MEDIASERVO_IMGUI_SHELL_TEXTURE_HPP */
