use shared_global;
use std::collections::VecDeque;
use std::collections::HashMap;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use sensor_sim::traits::Sensor;
use sensor_sim::thermometer::ThermoReading;
use sensor_sim::accelerometer::AccelReading;
use sensor_sim::force_sensor::ForceReading;
use shared_global::SensorType;
use shared_global::UnifiedReading;
use std::sync::atomic::AtomicBool;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::time::Instant;
use std::io::{BufRead, BufReader, Read};

const CRITICAL: f64 = 0.2;
const WARNING: f64 = 0.25;

/// Sensor statistics for a single sensor.
#[derive(Debug, Clone)]
struct SensorStats {
    rate: u32,
    last_available: usize,
}

/// A bounded buffer manager that stores sensor readings and provides
/// concurrent access for multiple producer threads (via push) and a single
/// consumer (via pop). It supports dynamic capacity expansion under high load.
pub struct BufferManager<T> {
    buffer: Mutex<VecDeque<T>>,
    capacity: AtomicUsize,
    not_empty: Condvar,
    pub total_writes: AtomicUsize,
    total_reads: AtomicUsize,
    overflow_warnings: AtomicUsize,
    running: AtomicBool,
    threads: Mutex<Vec<JoinHandle<()>>>,
    start_time: Instant,
    sensor_stats: Mutex<HashMap<String, SensorStats>>, // Maps sensor id to its latest stats
    sensor_rates: Mutex<HashMap<String, u32>>,         // Maps sensor id to its configured rate
}

impl BufferManager<UnifiedReading> {
    /// Creates a new `BufferManager` with the given initial capacity.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be > 0");
        Self {
            buffer: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity: AtomicUsize::new(capacity),
            not_empty: Condvar::new(),
            total_writes: AtomicUsize::new(0),
            total_reads: AtomicUsize::new(0),
            overflow_warnings: AtomicUsize::new(0),
            running: AtomicBool::new(true),
            threads: Mutex::new(Vec::new()),
            start_time: Instant::now(),
            sensor_stats: Mutex::new(HashMap::new()),
            sensor_rates: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the current capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.capacity.load(Ordering::Relaxed)
    }

    /// Returns the current number of elements in the buffer.
    pub fn len(&self) -> usize {
        let buf = self.buffer.lock().unwrap();
        buf.len()
    }

    /// Registers a new pipe reader thread that reads lines from a child process's stdout.
    ///
    /// This method spawns a thread that continuously reads lines from the provided `reader`
    /// (typically a pipe from a child process). Lines are expected to follow a simple protocol:
    /// - `RATE,<rate>`: informs the buffer of the sensor's sampling rate.
    /// - `AVAIL,<available>`: reports the sensor's internal queue occupancy.
    /// - `T,<value>`: temperature reading.
    /// - `A,<x>,<y>,<z>`: accelerometer reading.
    /// - `F,<x>,<y>,<z>`: force sensor reading.
    ///
    /// The thread accumulates readings into batches (only for efficiency of parsing, not for
    /// batch pushing) and pushes them one by one using `push()`. The batch size is dynamically
    /// adjusted based on the sensor's reported available space.
    ///
    /// # Arguments
    /// * `sensor_id` - A unique identifier for the sensor (used for statistics).
    /// * `reader` - An object implementing `Read + Send + 'static`, typically the stdout of a child process.
    pub fn register_pipe_reader<R: Read + Send + 'static>(
        self: &Arc<Self>,
        sensor_id: String,
        reader: R,
    ) {
        let manager = self.clone();
        let handle = thread::spawn(move || {
            let buf_reader = BufReader::new(reader);
            let mut local_buffer = Vec::with_capacity(100); // Local batch buffer
            let mut batch_size = 20; // Default batch size
            let mut rate = 0; // Sensor rate (optional, not currently used)

            for line in buf_reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(e) => {
                        eprintln!("Error reading pipe: {}", e);
                        break;
                    }
                };
                let parts: Vec<&str> = line.split(',').collect();
                if parts.is_empty() {
                    continue;
                }
                match parts[0] {
                    "RATE" => {
                        if parts.len() == 2 {
                            rate = parts[1].parse().unwrap_or(0);
                            let mut rates = manager.sensor_rates.lock().unwrap();
                            rates.insert(sensor_id.clone(), rate);
                        }
                    }
                    "AVAIL" => {
                        if parts.len() == 2 {
                            let avail: usize = parts[1].parse().unwrap_or(0);
                            // Dynamically adjust batch size based on available space
                            // Assume sensor internal queue capacity is 128
                            let new_batch_size = if avail < 20 {
                                80  // Urgent: little free space, read many
                            } else if avail < 50 {
                                40  // Medium
                            } else {
                                20  // Normal
                            };
                            if new_batch_size != batch_size {
                                batch_size = new_batch_size;
                                // Ensure local buffer capacity is sufficient
                                if local_buffer.capacity() < batch_size {
                                    local_buffer.reserve(batch_size - local_buffer.capacity());
                                }
                            }
                        }
                    }
                    "T" => {
                        if parts.len() == 2 {
                            let val: f32 = parts[1].parse().unwrap_or(0.0);
                            let reading = UnifiedReading::Thermo(ThermoReading {
                                temperature_celsius: val,
                            });
                            local_buffer.push(reading);
                        }
                    }
                    "A" => {
                        if parts.len() == 4 {
                            let x: f32 = parts[1].parse().unwrap_or(0.0);
                            let y: f32 = parts[2].parse().unwrap_or(0.0);
                            let z: f32 = parts[3].parse().unwrap_or(0.0);
                            let reading = UnifiedReading::Accel(AccelReading {
                                acceleration_x: x,
                                acceleration_y: y,
                                acceleration_z: z,
                            });
                            local_buffer.push(reading);
                        }
                    }
                    "F" => {
                        if parts.len() == 4 {
                            let x: f32 = parts[1].parse().unwrap_or(0.0);
                            let y: f32 = parts[2].parse().unwrap_or(0.0);
                            let z: f32 = parts[3].parse().unwrap_or(0.0);
                            let reading = UnifiedReading::Force(ForceReading {
                                force_x: x,
                                force_y: y,
                                force_z: z,
                            });
                            local_buffer.push(reading);
                        }
                    }
                    _ => {}
                }

                // If local buffer has reached batch size, push items one by one
                if local_buffer.len() >= batch_size {
                    let mut rejected_count = 0;
                    for item in local_buffer.drain(..) {
                        if let Err(item) = manager.push(item) {
                            // Buffer full, count as rejected and discard (no retry)
                            rejected_count += 1;
                        }
                    }
                    if rejected_count > 0 {
                        eprintln!("Buffer full, dropped {} readings", rejected_count);
                    }
                    // Reallocate with capacity = batch_size to avoid frequent resizing
                    local_buffer = Vec::with_capacity(batch_size);
                }
            }

            // Thread ending: push any remaining data one by one
            if !local_buffer.is_empty() {
                let mut rejected_count = 0;
                for item in local_buffer.drain(..) {
                    if let Err(item) = manager.push(item) {
                        rejected_count += 1;
                    }
                }
                if rejected_count > 0 {
                    eprintln!("Buffer full, dropped {} readings", rejected_count);
                }
            }
        });
        self.threads.lock().unwrap().push(handle);
    }

    /// Returns the estimated emergency time (in seconds) for a given sensor.
    ///
    /// Emergency is defined as `(127 - last_available) / rate`, representing the time
    /// until the sensor's internal queue becomes full. Returns `None` if the sensor is
    /// unknown.
    pub fn get_emergency(&self, sensor_id: &str) -> Option<f64> {
        let stats_map = self.sensor_stats.lock().unwrap();
        stats_map.get(sensor_id).map(|s| {
            if s.rate == 0 {
                f64::INFINITY
            } else {
                (127 - s.last_available) as f64 / s.rate as f64
            }
        })
    }

    /// Prints emergency levels for all registered sensors (for debugging).
    pub fn print_all_emergencies(&self) {
        let stats_map = self.sensor_stats.lock().unwrap();
        for (id, stat) in stats_map.iter() {
            let emergency = if stat.rate == 0 {
                f64::INFINITY
            } else {
                (127 - stat.last_available) as f64 / stat.rate as f64
            };
            println!("Sensor {}: rate={}, avail={}, emergency={:.3}s", id, stat.rate, stat.last_available, emergency);
        }
    }

    /// Prints statistics about the buffer (size, writes, reads, rates, warnings, threads).
    pub fn print_stats(&self) {
        let current_size = self.len();
        let capacity = self.capacity();
        let utilization = if capacity > 0 {
            current_size as f64 / capacity as f64 * 100.0
        } else {
            0.0
        };
        let total_writes = self.total_writes.load(Ordering::Relaxed);
        let total_reads = self.total_reads.load(Ordering::Relaxed);
        let overflow_warnings = self.overflow_warnings.load(Ordering::Relaxed);
        let thread_count = self.threads.lock().unwrap().len();

        let elapsed = self.start_time.elapsed().as_secs_f64();
        let write_rate = if elapsed > 0.0 {
            total_writes as f64 / elapsed
        } else {
            0.0
        };
        let read_rate = if elapsed > 0.0 {
            total_reads as f64 / elapsed
        } else {
            0.0
        };

        println!(
            "Buffer Stats: size={}/{}, util={:.1}%, writes={}, reads={}, write_rate={:.0}/s, read_rate={:.0}/s, warnings={}, threads={}",
            current_size,
            capacity,
            utilization,
            total_writes,
            total_reads,
            write_rate,
            read_rate,
            overflow_warnings,
            thread_count
        );
    }

    /// Shuts down all background threads (both pipe readers and sensor algorithm threads).
    /// Waits for them to finish.
    pub fn shutdown(&self) {
        self.running.store(false, Ordering::Relaxed);

        // Take all handles out of the mutex
        let handles = {
            let mut threads = self.threads.lock().unwrap();
            threads.drain(..).collect::<Vec<_>>()
        };

        // Now lock is released, we can safely join
        for handle in handles {
            thread::sleep(Duration::from_millis(1)); // Give threads a moment to exit
            if let Err(e) = handle.join() {
                eprintln!("Thread join failed: {:?}", e);
            }
        }
    }

    /// Returns `true` if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        let buf = self.buffer.lock().unwrap();
        buf.is_empty()
    }

    /// Returns `true` if the buffer is full (size >= capacity).
    pub fn is_full(&self) -> bool {
        let buf = self.buffer.lock().unwrap();
        buf.len() >= self.capacity.load(Ordering::Relaxed)
    }

    /// Attempts to push a single item into the buffer.
    /// Returns `Ok(())` on success, or `Err(item)` if the buffer is full.
    pub fn push(&self, item: UnifiedReading) -> Result<(), UnifiedReading> {
        let mut buf = self.buffer.lock().unwrap();
        let current_cap = self.capacity.load(Ordering::Relaxed);
        if buf.len() >= current_cap {
            return Err(item);
        }
        buf.push_back(item);
        let current_len = buf.len();
        self.total_writes.fetch_add(1, Ordering::Relaxed);
        // Check if utilization exceeds 90%
        if current_len > current_cap * 90 / 100 {
            self.overflow_warnings.fetch_add(1, Ordering::Relaxed);
            let sensor_count = self.threads.lock().unwrap().len();
            const SENSOR_LESS: usize = 3;
            const SENSOR_MORE: usize = 150;
            if sensor_count > SENSOR_LESS && sensor_count < SENSOR_MORE {
                let new_cap = current_cap + 30000;
                self.capacity.store(new_cap, Ordering::Relaxed);
            } else if sensor_count >= SENSOR_MORE {
                let new_cap = current_cap + 80000;
                self.capacity.store(new_cap, Ordering::Relaxed);
            }
        }
        self.not_empty.notify_one();
        Ok(())
    }

    /// Blocks until an item is available, then returns it.
    pub fn pop(&self) -> UnifiedReading {
        let mut buf = self.buffer.lock().unwrap();

        // Wait while buffer is empty
        while buf.is_empty() {
            buf = self.not_empty.wait(buf).unwrap();
        }

        let item = buf.pop_front().unwrap();
        self.total_reads.fetch_add(1, Ordering::Relaxed);

        item
    }

    /// Attempts to pop an item, blocking for at most `timeout`.
    /// Returns `None` if the timeout expires and no item is available.
    pub fn pop_timeout(&self, timeout: Duration) -> Option<UnifiedReading> {
        // 获取锁，如果中毒则返回 None（避免 panic 和进一步中毒）
        let mut buf = match self.buffer.lock() {
            Ok(guard) => guard,
            Err(e) => {
                eprintln!("Buffer mutex poisoned, returning None");
                return None;
            }
        };

        if buf.is_empty() {
            // 等待条件变量，如果中毒则返回 None
            match self.not_empty.wait_timeout(buf, timeout) {
                Ok((new_buf, wait_result)) => {
                    buf = new_buf;
                    if wait_result.timed_out() {
                        return None;
                    }
                }
                Err(e) => {
                    eprintln!("Condition variable wait poisoned, returning None");
                    return None;
                }
            }
        }

        // 弹出数据，使用 ? 安全处理（理论上非空，但预防万一）
        let item = buf.pop_front()?;
        self.total_reads.fetch_add(1, Ordering::Relaxed);
        Some(item)
    }

    /// Non-blocking pop: returns `Some(item)` if available, else `None`.
    pub fn try_pop(&self) -> Option<UnifiedReading> {
        let mut buf = self.buffer.lock().unwrap();
        buf.pop_front()
    }

    /// Returns a reference to the front item without removing it, or `None` if empty.
    pub fn peek(&self) -> Option<UnifiedReading> {
        let buf = self.buffer.lock().unwrap();
        buf.front().cloned()
    }
}