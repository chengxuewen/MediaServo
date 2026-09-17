#include "imgui_shell/texture.hpp"

#include <SDL3/SDL.h>

#include <cstring>

namespace imgui_shell {

void FrameStaging::push(const void* i420, size_t len, uint32_t w, uint32_t h) {
    std::lock_guard<std::mutex> lk(mu_);
    slot_.resize(len);
    std::memcpy(slot_.data(), i420, len);
    w_ = w;
    h_ = h;
    ready_ = true;
    pushed_.fetch_add(1, std::memory_order_relaxed);
}

bool FrameStaging::pop(std::vector<uint8_t>& out, uint32_t* w, uint32_t* h) {
    std::lock_guard<std::mutex> lk(mu_);
    if (!ready_) return false;
    ready_ = false;
    out.swap(slot_); // 主线程缓冲复用（swap 后 slot_ 持旧容量，下次 push 零分配）
    if (w) *w = w_;
    if (h) *h = h_;
    return true;
}

VideoTexture::~VideoTexture() { destroy(); }

void VideoTexture::destroy() {
    if (tex_) SDL_DestroyTexture(static_cast<SDL_Texture*>(tex_));
    tex_ = nullptr;
    w_ = h_ = 0;
}

bool VideoTexture::update(void* renderer, const void* i420, size_t len, uint32_t w, uint32_t h) {
    if (!i420 || w == 0 || h == 0 || len < static_cast<size_t>(w) * h * 3 / 2) return false;
    auto* rend = static_cast<SDL_Renderer*>(renderer);
    if (!rend) return false;
    if (w_ != w || h_ != h) {
        destroy();
        tex_ = SDL_CreateTexture(rend, SDL_PIXELFORMAT_IYUV,
                                 SDL_TEXTUREACCESS_STREAMING,
                                 static_cast<int>(w), static_cast<int>(h));
        if (!tex_) {
            SDL_Log("VideoTexture: CreateTexture %ux%u failed: %s", w, h, SDL_GetError());
            return false;
        }
        w_ = w;
        h_ = h;
    }
    const int pitch = static_cast<int>(w);
    const int half = static_cast<int>(w / 2);
    const uint8_t* planes = static_cast<const uint8_t*>(i420);
    const SDL_Rect area{0, 0, static_cast<int>(w), static_cast<int>(h)};
    if (!SDL_UpdateYUVTexture(static_cast<SDL_Texture*>(tex_), &area,
                              planes, pitch,                       // Y
                              planes + static_cast<size_t>(w) * h, half,   // U
                              planes + static_cast<size_t>(w) * h * 5 / 4, half)) { // V
        SDL_Log("VideoTexture: UpdateTexture failed: %s", SDL_GetError());
        return false;
    }
    ++frames_;
    return true;
}

} // namespace imgui_shell
