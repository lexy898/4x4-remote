//! Freed large buffers go back to the system once the mmap threshold is pinned.
//!
//! A test binary of its own: Cargo runs each integration-test binary as a separate process, so
//! nothing else allocates beside this test, also under plain `cargo test` (a unit test shares its
//! process with every other unit test there, and their allocations hide what is measured).
#![cfg(all(target_os = "linux", target_env = "gnu"))]

/// Bytes of anonymous memory the process holds, in KiB; what `free` gives back shows here.
fn rss_anon_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("RssAnon:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Allocates `bytes`, writes a byte into every page so that each is resident, and frees them.
fn touch_and_free(bytes: usize) {
    let mut buffer = vec![0_u8; bytes];
    for page in buffer.chunks_mut(4096) {
        if let Some(byte) = page.first_mut() {
            *byte = 1;
        }
    }
    std::hint::black_box(&buffer);
}

#[test]
fn freed_large_buffers_go_back_to_the_system() {
    assert!(x4_app::allocator::pin_mmap_threshold());
    let before = rss_anon_kib().expect("RssAnon in /proc/self/status");

    // As an add of a 16 MiB .torrent and then one of 8 MiB do on the service thread.
    std::thread::spawn(|| {
        touch_and_free(16 * 1024 * 1024);
        touch_and_free(8 * 1024 * 1024);
    })
    .join()
    .expect("the thread ends");

    let kept = rss_anon_kib()
        .expect("RssAnon in /proc/self/status")
        .saturating_sub(before);
    assert!(kept < 2048, "{kept} KiB stayed resident");
}
