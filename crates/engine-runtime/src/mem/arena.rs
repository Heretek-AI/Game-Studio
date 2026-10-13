use std::{
    alloc::{alloc, dealloc, Layout},
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

pub struct BumpArena {
    base: NonNull<u8>,
    capacity: usize,
    cursor: AtomicUsize,
    high_water: AtomicUsize,
    layout: Layout,
}

unsafe impl Send for BumpArena {}
unsafe impl Sync for BumpArena {}

impl BumpArena {
    pub fn new(capacity_bytes: usize, align: usize) -> Self {
        assert!(capacity_bytes > 0, "arena capacity must be non-zero");
        assert!(align.is_power_of_two(), "arena align must be power of two");
        let layout = Layout::from_size_align(capacity_bytes, align)
            .expect("invalid arena layout for budget label");
        let raw = unsafe { alloc(layout) };
        let base = NonNull::new(raw).unwrap_or_else(|| {
            panic!("arena OOM: failed to allocate {capacity_bytes} bytes (budget label: frame)")
        });
        Self {
            base,
            capacity: capacity_bytes,
            cursor: AtomicUsize::new(0),
            high_water: AtomicUsize::new(0),
            layout,
        }
    }

    pub fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut current = self.cursor.load(Ordering::Relaxed);
        loop {
            let aligned = current
                .checked_add(layout.align() - 1)
                .map(|v| v & !(layout.align() - 1));
            let Some(aligned) = aligned else {
                return std::ptr::null_mut();
            };
            let Some(end) = aligned.checked_add(layout.size()) else {
                return std::ptr::null_mut();
            };
            if end > self.capacity {
                return std::ptr::null_mut();
            }
            match self.cursor.compare_exchange_weak(
                current,
                end,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.high_water.fetch_max(end, Ordering::Relaxed);
                    let ptr = unsafe { self.base.as_ptr().add(aligned) };
                    return ptr;
                }
                Err(actual) => current = actual,
            }
        }
    }

    pub fn reset(&self) {
        #[cfg(debug_assertions)]
        {
            let used = self.cursor.load(Ordering::Relaxed);
            if used > 0 {
                unsafe {
                    std::ptr::write_bytes(self.base.as_ptr(), 0xCD, used.min(self.capacity));
                }
            }
        }
        self.cursor.store(0, Ordering::Relaxed);
        std::sync::atomic::compiler_fence(Ordering::SeqCst);
    }

    pub fn used(&self) -> usize {
        self.cursor.load(Ordering::Relaxed)
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn high_water(&self) -> usize {
        self.high_water.load(Ordering::Relaxed)
    }

    pub fn high_water_ratio(&self) -> f64 {
        if self.capacity == 0 {
            return 0.0;
        }
        self.high_water() as f64 / self.capacity as f64
    }
}

impl Drop for BumpArena {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.base.as_ptr(), self.layout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_and_reset_tracks_usage() {
        let arena = BumpArena::new(1024, 16);
        assert_eq!(arena.used(), 0);
        let layout = Layout::from_size_align(64, 16).unwrap();
        let ptr = arena.alloc(layout);
        assert!(!ptr.is_null());
        assert!(arena.used() >= 64);
        arena.reset();
        assert_eq!(arena.used(), 0);
        assert!(arena.high_water() >= 64);
    }

    #[test]
    fn exhaustion_returns_null() {
        let arena = BumpArena::new(64, 16);
        let big = Layout::from_size_align(128, 16).unwrap();
        assert!(arena.alloc(big).is_null());
    }
}
