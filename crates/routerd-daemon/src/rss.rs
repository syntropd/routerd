use rustix::param::page_size;
use std::fs;
use tracing::warn;

pub const RSS_TARGET_MAX_BYTES: u64 = 15 * 1024 * 1024; // 15MB

#[derive(Debug, Clone, Copy)]
pub struct MemoryStats {
    pub rss_bytes: u64,
    #[allow(dead_code)]
    pub virt_bytes: u64,
    pub page_size: usize,
}

extern "C" {
    fn malloc_trim(pad: usize) -> i32;
}

/// Request glibc allocator to return unused heap memory pages back to the kernel immediately.
pub fn trim_memory_pages() {
    unsafe {
        let _ = malloc_trim(0);
    }
}

impl MemoryStats {
    pub fn read_current() -> Self {
        let ps = page_size();
        if let Ok(statm) = fs::read_to_string("/proc/self/statm") {
            let mut parts = statm.split_whitespace();
            let size_pages: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
            let resident_pages: u64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);

            let rss = resident_pages * ps as u64;
            let virt = size_pages * ps as u64;

            if rss > RSS_TARGET_MAX_BYTES {
                trim_memory_pages();
                warn!(
                    "Memory RSS threshold exceeded: {:.2} MB > 15 MB target, performed page trim",
                    rss as f64 / (1024.0 * 1024.0)
                );
            }

            Self {
                rss_bytes: rss,
                virt_bytes: virt,
                page_size: ps,
            }
        } else {
            Self {
                rss_bytes: 0,
                virt_bytes: 0,
                page_size: ps,
            }
        }
    }

    pub fn rss_mb(&self) -> f64 {
        self.rss_bytes as f64 / (1024.0 * 1024.0)
    }
}
