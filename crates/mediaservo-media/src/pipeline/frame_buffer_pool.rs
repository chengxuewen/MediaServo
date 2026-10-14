//! FrameBufferPool — I420 frame buffer pool (memory reuse, avoids per-frame allocation).
//!
//! In high-fps pipelines every `I420Buffer::new` creates heap allocation pressure (GC/allocator jitter).
//! The pool buckets buffers by (width, height) and reuses recycled buffers.
//!
//! Mirrors OpenCTK `video_frame_buffer_pool.hpp`: Get → reuse (allocate when no idle buffer),
//! Recycle → return. Not thread safe (the caller guarantees single-threaded use or external synchronization).

use std::collections::HashMap;

use crate::base::buffer::{I420Buffer, VideoBuffer};

/// I420 frame buffer pool.
#[derive(Debug, Default)]
pub struct FrameBufferPool {
    /// key = (width, height), value = idle buffer stack.
    free: HashMap<(u32, u32), Vec<I420Buffer>>,
    /// Pool cap (per size); recycling beyond the cap drops the buffer directly.
    max_pool_size_per_size: usize,
}

impl FrameBufferPool {
    /// Creates the pool, default cap 4 per size.
    pub fn new() -> Self {
        Self::with_limit(4)
    }

    /// Creates the pool and sets the idle cap per size.
    pub fn with_limit(max_pool_size_per_size: usize) -> Self {
        Self { free: HashMap::new(), max_pool_size_per_size }
    }

    /// Gets an I420 buffer of (width, height) (reuse or new allocation).
    pub fn get(&mut self, width: u32, height: u32) -> I420Buffer {
        self.free
            .get_mut(&(width, height))
            .and_then(|stack| stack.pop())
            .unwrap_or_else(|| I420Buffer::new(width, height))
    }

    /// Returns a buffer for reuse; drops it if the idle stack exceeds the cap (avoids unbounded pool growth).
    pub fn recycle(&mut self, buffer: I420Buffer) {
        let key = (buffer.width(), buffer.height());
        let stack = self.free.entry(key).or_default();
        if stack.len() < self.max_pool_size_per_size {
            stack.push(buffer);
        }
        // Over cap: drop the buffer directly (release memory)
    }

    /// Current total number of idle buffers.
    pub fn idle_count(&self) -> usize {
        self.free.values().map(|v| v.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_recycles_same_size_buffer() {
        let mut pool = FrameBufferPool::new();
        let b = pool.get(640, 480);
        assert_eq!(b.width(), 640);
        assert_eq!(b.height(), 480);
        pool.recycle(b);
        assert_eq!(pool.idle_count(), 1);
        // Get again → reuse (idle shrinks, no new allocation)
        let b2 = pool.get(640, 480);
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(b2.width(), 640);
    }

    #[test]
    fn different_sizes_have_separate_buckets() {
        let mut pool = FrameBufferPool::new();
        let b1 = pool.get(640, 480);
        let b2 = pool.get(320, 240);
        pool.recycle(b1);
        pool.recycle(b2);
        assert_eq!(pool.idle_count(), 2);
        // Different sizes do not share buffers
        let _ = pool.get(640, 480);
        assert_eq!(pool.idle_count(), 1);
    }

    #[test]
    fn recycle_drops_when_pool_exceeds_limit() {
        let mut pool = FrameBufferPool::with_limit(2);
        let b1 = pool.get(100, 100);
        let b2 = pool.get(100, 100);
        let b3 = pool.get(100, 100);
        pool.recycle(b1);
        pool.recycle(b2);
        pool.recycle(b3); // Third one → dropped
        assert_eq!(pool.idle_count(), 2);
    }

    #[test]
    fn get_creates_new_when_pool_empty() {
        let mut pool = FrameBufferPool::new();
        let b = pool.get(16, 16);
        assert_eq!(b.width(), 16);
        assert_eq!(pool.idle_count(), 0);
    }
}
