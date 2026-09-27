//! Lock-contention free SPSC ring buffer for audio data.
//!
//! Used to pass decoded PCM data from the decoder thread to the output thread.
//! Producer (decode) and Consumer (output) have independent locks to avoid contention.

use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;
use std::sync::Arc;
use std::sync::Mutex;

/// Thread-safe ring buffer for PCM samples.
/// Producer (decode) and Consumer (output) locks are separated to eliminate contention.
pub struct RingBuffer {
    producer: Arc<Mutex<<HeapRb<f32> as Split>::Prod>>,
    consumer: Arc<Mutex<<HeapRb<f32> as Split>::Cons>>,
    capacity: usize,
}

impl RingBuffer {
    /// Create a new ring buffer with the given capacity in samples.
    pub fn new(capacity: usize) -> Self {
        let rb = HeapRb::<f32>::new(capacity);
        let (prod, cons) = rb.split();

        Self {
            producer: Arc::new(Mutex::new(prod)),
            consumer: Arc::new(Mutex::new(cons)),
            capacity,
        }
    }

    /// Write samples to the buffer.
    /// Returns the number of samples actually written.
    pub fn write(&self, samples: &[f32]) -> usize {
        let mut prod = self.producer.lock().unwrap();
        prod.push_slice(samples)
    }

    /// Read samples from the buffer.
    /// Returns the number of samples actually read.
    /// Returns 0 if the consumer mutex is poisoned (graceful degradation).
    pub fn read(&self, buf: &mut [f32]) -> usize {
        match self.consumer.lock() {
            Ok(mut cons) => cons.pop_slice(buf),
            Err(_) => 0, // poisoned — return empty to keep callback alive
        }
    }

    /// Number of samples currently in the buffer (occupied by consumer).
    pub fn available(&self) -> usize {
        let cons = self.consumer.lock().unwrap();
        cons.occupied_len()
    }

    /// Free space remaining (available for producer).
    pub fn free_space(&self) -> usize {
        let prod = self.producer.lock().unwrap();
        prod.vacant_len()
    }

    /// Total capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clear all data from the buffer (blocking).
    /// Blocks until the consumer lock is available, then clears.
    pub fn clear(&self) {
        let mut cons = self.consumer.lock().unwrap();
        let occupied = cons.occupied_len();
        if occupied > 0 {
            cons.skip(occupied);
        }
    }

    /// Try to clear all data from the buffer (non-blocking).
    /// Always non-blocking, only clears if lock can be acquired.
    pub fn try_clear(&self) -> bool {
        if let Ok(mut cons) = self.consumer.try_lock() {
            let occupied = cons.occupied_len();
            cons.skip(occupied);
            true
        } else {
            false
        }
    }

    /// Drain excess samples, keeping at most `keep` samples in the buffer.
    /// Used for speed change to minimize old-speed audio playback delay.
    /// Retries up to 20 times with 100µs sleeps to handle output thread contention.
    pub fn drain_to(&self, keep: usize) {
        for _ in 0..20 {
            if let Ok(mut cons) = self.consumer.try_lock() {
                let occupied = cons.occupied_len();
                if occupied > keep {
                    cons.skip(occupied - keep);
                }
                return;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        // Failed to acquire lock after retries — output thread is busy.
        // The speed change will still take effect, just with slightly more delay.
    }

    /// Check if the buffer is empty.
    /// Uses blocking lock — Consumer lock is only held briefly by output callback.
    pub fn is_empty(&self) -> bool {
        let cons = self.consumer.lock().unwrap();
        cons.is_empty()
    }
}

impl Clone for RingBuffer {
    fn clone(&self) -> Self {
        Self {
            producer: self.producer.clone(),
            consumer: self.consumer.clone(),
            capacity: self.capacity,
        }
    }
}

unsafe impl Send for RingBuffer {}
unsafe impl Sync for RingBuffer {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_read() {
        let buf = RingBuffer::new(1024);
        let samples: Vec<f32> = (0..100).map(|i| i as f32).collect();

        let written = buf.write(&samples);
        assert_eq!(written, 100);

        let mut read_buf = vec![0.0f32; 200];
        let read = buf.read(&mut read_buf);
        assert_eq!(read, 100);

        for (i, &v) in read_buf.iter().enumerate().take(100) {
            assert_eq!(v, i as f32);
        }
    }

    #[test]
    fn test_clear() {
        let buf = RingBuffer::new(1024);
        let samples = vec![1.0f32; 500];
        buf.write(&samples);
        assert_eq!(buf.available(), 500);

        buf.clear();
        // After clear, available may not be exactly 0 if consumer lock was held
        // (try_clear behavior). But in test it should be 0 since no contention.
        assert!(buf.is_empty() || buf.available() == 0);
    }

    #[test]
    fn test_overflow() {
        let buf = RingBuffer::new(100);
        let samples = vec![1.0f32; 200];
        let written = buf.write(&samples);
        assert_eq!(written, 100); // Should only write up to capacity
    }
}
