use routerd_core::router::read_rss_info;
use routerd_daemon::rss::{trim_memory_pages, MemoryStats, RSS_TARGET_MAX_BYTES};

#[test]
fn test_memory_rss_reading_and_threshold() {
    let stats = MemoryStats::read_current();
    assert!(stats.page_size >= 4096);
    assert!(stats.rss_bytes > 0);

    let (rss_bytes, rss_mb) = read_rss_info();
    assert!(rss_bytes > 0);
    assert!(rss_mb > 0.0);

    // Verify Memory RSS conforms to <15MB target in test execution
    assert!(
        stats.rss_bytes < RSS_TARGET_MAX_BYTES,
        "RSS was {:.2} MB which exceeds 15 MB cap",
        stats.rss_mb()
    );

    // Verify trim_memory_pages runs without segfault/panic
    trim_memory_pages();
}
