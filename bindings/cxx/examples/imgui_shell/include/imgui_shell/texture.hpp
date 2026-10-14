/* imgui_shell texture layer — the cross-thread carrier from I420 frames to an SDL texture (W3b).
 *
 * Threading model (aligned with the SDK frame callback's "valid only inside the callback" contract):
 *   pump thread → FrameStaging::push()   (latest-only: a new frame overwrites an unconsumed old one = natural frame dropping, never blocking)
 *   main thread → pop() → VideoTexture::update() → texture_id() for ImGui::Image
 *
 * SDL_Renderer path only (imgui_impl_sdlrenderer3 uses SDL_Texture* as ImTextureID).
 * The header deliberately does not depend on SDL types (void* handles), so include order is free.
 */
#ifndef MEDIASERVO_IMGUI_SHELL_TEXTURE_HPP
#define MEDIASERVO_IMGUI_SHELL_TEXTURE_HPP

#include <atomic>
#include <cstdint>
#include <mutex>
#include <vector>

namespace imgui_shell {

/// The latest-frame slot from the pump thread to the main thread (bounded at one frame copy; no queue, no backpressure).
class FrameStaging {
public:
    void push(const void* i420, size_t len, uint32_t w, uint32_t h);

    /// Take the unconsumed frame (false = no new frame). Reuses out's capacity (zero steady-state allocation per frame).
    bool pop(std::vector<uint8_t>& out, uint32_t* w, uint32_t* h);

    uint64_t pushed() const { return pushed_.load(std::memory_order_relaxed); }

private:
    std::mutex mu_;
    std::vector<uint8_t> slot_;
    uint32_t w_ = 0, h_ = 0;
    bool ready_ = false;
    std::atomic<uint64_t> pushed_{0};
};

/// The main-thread video texture station (update rebuilds on size change; black frames are not drawn — it just reports id=0).
class VideoTexture {
public:
    VideoTexture() = default;
    ~VideoTexture();
    VideoTexture(const VideoTexture&) = delete;
    VideoTexture& operator=(const VideoTexture&) = delete;

    /// renderer = the main-thread SDL_Renderer* (App::sdl_renderer()).
    /// i420 = contiguous planes Y(w*h)+U(w*h/4)+V(w*h/4) (fed directly in the SDL IYUV layout, zero copy).
    bool update(void* renderer, const void* i420, size_t len, uint32_t w, uint32_t h);

    /// The texture handle for ImGui::Image (SDL_Texture*; nullptr until established).
    void* texture_id() const { return tex_; }
    uint32_t width() const { return w_; }
    uint32_t height() const { return h_; }
    uint64_t frames() const { return frames_; }

private:
    void destroy();

    void* tex_ = nullptr; // SDL_Texture* (the renderer is fetched live on the main thread via SDL_GetRenderer)
    uint32_t w_ = 0, h_ = 0;
    uint64_t frames_ = 0;
};

} // namespace imgui_shell

#endif /* MEDIASERVO_IMGUI_SHELL_TEXTURE_HPP */
