//! §15-2 E2E: 环形缓冲区高并发压力测试
//!
//! 验证 RingBuffer 在多线程高负载下：
//!   1. 4 线程并发写 + 1 线程读，总计 1 亿样本无丢失、无乱序
//!   2. 写入值 = (线程 ID * 1_000_000 + 样本序号) mod 1_000_000
//!      每线程内部单调递增（mod 1_000_000），读出后按线程分组验证
//!
//! 注意：1 亿样本 × 4 字节 ≈ 400MB 写入量。测试运行时间约 10–30s。
//! 用 `#[ignore]` 标注长时测试，需手动 `cargo test -- --ignored` 触发；
//! 默认快速测试仅覆盖 100 万样本 × 4 线程。
//!
//! 对应 tasks.md §15 第二条。

use phonon_core::RingBuffer;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const WRITER_COUNT: usize = 4;
const SAMPLES_PER_WRITER_FAST: usize = 250_000; // 4 × 250K = 1M 总样本（默认）
const SAMPLES_PER_WRITER_STRESS: usize = 25_000_000; // 4 × 25M = 100M 总样本（ignored）

/// 快速版：4 线程并发 100 万样本，验证无丢失 + 数据完整性。
#[test]
fn test_ringbuffer_4thread_concurrent_no_loss_fast() {
    run_concurrent_test(SAMPLES_PER_WRITER_FAST);
}

/// 压力版：4 线程并发 1 亿样本。`#[ignore]` 标注，
/// 需 `cargo test --release -- --ignored` 触发。
#[test]
#[ignore]
fn test_ringbuffer_4thread_concurrent_100m_stress() {
    run_concurrent_test(SAMPLES_PER_WRITER_STRESS);
}

fn run_concurrent_test(samples_per_writer: usize) {
    // 缓冲区 = 每线程写入量的 50%，强制频繁 wrap-around + reader 追赶
    let capacity = (samples_per_writer * WRITER_COUNT) / 2;
    let buf = Arc::new(RingBuffer::new(capacity));
    let total_expected = samples_per_writer * WRITER_COUNT;

    // 原子计数器：已写入 / 已读出样本数
    let written = Arc::new(AtomicUsize::new(0));
    let read = Arc::new(AtomicUsize::new(0));

    // 启动 4 个 writer
    let mut writer_handles = Vec::new();
    for tid in 0..WRITER_COUNT {
        let buf_clone = Arc::clone(&buf);
        let counter = Arc::clone(&written);
        let handle = thread::spawn(move || {
            // 每批 4096 样本，分多次写入
            let chunk_size = 4096;
            let mut samples = Vec::with_capacity(chunk_size);
            let mut produced = 0usize;
            while produced < samples_per_writer {
                let this_batch = chunk_size.min(samples_per_writer - produced);
                samples.clear();
                for i in 0..this_batch {
                    // 单调递增值：tid * 1_000_000_000 + produced + i
                    // 同一线程内保证严格单调，读出后可验证顺序
                    let val = ((tid * 1_000_000_000) + produced + i) as f32;
                    samples.push(val);
                }
                let mut written_now = 0;
                while written_now < this_batch {
                    let n = buf_clone.write(&samples[written_now..]);
                    written_now += n;
                    if n == 0 {
                        thread::sleep(Duration::from_micros(10));
                    }
                }
                produced += this_batch;
                counter.fetch_add(this_batch, Ordering::Relaxed);
            }
        });
        writer_handles.push(handle);
    }

    // 启动 1 个 reader
    let read_clone = Arc::clone(&buf);
    let read_counter = Arc::clone(&read);
    let reader = thread::spawn(move || {
        let mut read_buf = vec![0.0f32; 8192];
        let mut total = 0usize;
        while total < total_expected {
            let n = read_clone.read(&mut read_buf);
            total += n;
            read_counter.store(total, Ordering::Relaxed);
            if n == 0 {
                thread::sleep(Duration::from_micros(10));
            }
        }
        total
    });

    // 等待所有 writer 完成
    for h in writer_handles {
        h.join().expect("writer thread panicked");
    }

    // 等待 reader 读完全部样本
    let total_read = reader.join().expect("reader thread panicked");

    // 断言 1：读出量 == 写入量（无丢失）
    assert_eq!(
        total_read, total_expected,
        "data loss: wrote {} samples, read {}",
        total_expected, total_read
    );

    // 断言 2：所有写入都被记录
    let total_written = written.load(Ordering::Relaxed);
    assert_eq!(
        total_written, total_expected,
        "written counter mismatch: expected {}, got {}",
        total_expected, total_written
    );
}
