use sensor_sim::accelerometer::AccelReading;
use sensor_sim::force_sensor::ForceReading;
use sensor_sim::thermometer::ThermoReading;
use sensor_sim::traits::Sensor;
use shared_global;
use shared_global::UnifiedReading;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::AtomicBool;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Instant;
use std::time::Duration;
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
    /// batch pushing) and pushes them one by one using `push_batch()`.
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
            let mut rate = 0;
            let mut last_avail = 128;
            let mut local_buffer = Vec::with_capacity(100);
            let  batch_size = 20;
            let mut sleep_duration = Duration::from_micros(100);

            for line in buf_reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(e) => {
                        eprintln!("Error reading pipe");
                        break;
                    }
                };
                let parts: Vec<&str> = line.split(',').collect();
                match parts[0] {
                    "RATE" => {
                        if parts.len() == 2 {
                            rate = parts[1].parse().unwrap();
                        }
                    }
                    "AVAIL" => {
                        if parts.len() == 2 {
                            last_avail = parts[1].parse().unwrap();
                            if rate > 0 {
                                let emergence = (128 - last_avail) as f64 / rate as f64;
                                sleep_duration = if emergence < 0.08 {
                                    Duration::from_micros(0)
                                } else if emergence < 0.2 {
                                    Duration::from_micros(50)
                                } else {
                                    Duration::from_micros(200)
                                };
                            }
                        }
                    }
                    "T" => {
                        if parts.len() == 2 {
                            let val: f32 = parts[1].parse().unwrap();
                            let reading = UnifiedReading::Thermo(
                                sensor_id.clone(),
                                ThermoReading { temperature_celsius: val },
                            );
                            local_buffer.push(reading);
                        }
                    }
                    "A" => {
                        if parts.len() == 4 {
                            let x: f32 = parts[1].parse().unwrap();
                            let y: f32 = parts[2].parse().unwrap();
                            let z: f32 = parts[3].parse().unwrap();
                            let reading = UnifiedReading::Accel(
                                sensor_id.clone(),
                                AccelReading {
                                    acceleration_x: x,
                                    acceleration_y: y,
                                    acceleration_z: z,
                                },
                            );
                            local_buffer.push(reading);
                        }
                    }
                    "F" => {
                        if parts.len() == 4 {
                            let x: f32 = parts[1].parse().unwrap();
                            let y: f32 = parts[2].parse().unwrap();
                            let z: f32 = parts[3].parse().unwrap();
                            let reading = UnifiedReading::Force(
                                sensor_id.clone(),
                                ForceReading {
                                    force_x: x,
                                    force_y: y,
                                    force_z: z,
                                },
                            );
                            local_buffer.push(reading);
                        }
                    }
                    _ => {}
                }

                if local_buffer.len() >= batch_size {
                    let batch = std::mem::take(&mut local_buffer);
                    manager.push_batch(batch);
                    local_buffer = Vec::with_capacity(batch_size);
                }
            }
            thread::sleep(sleep_duration);
            if !local_buffer.is_empty() {
                manager.push_batch(local_buffer);
            }
        });
        self.threads.lock().unwrap().push(handle);
    }
    /// Only for comparasion testing usage.
    pub fn register_pipe_reader_no_edf<R: Read + Send + 'static>(
        self: &Arc<Self>,
        sensor_id: String,
        reader: R,
    ) {
        let manager = self.clone();
        let handle = thread::spawn(move || {
            let buf_reader = BufReader::new(reader);
            let mut local_buffer = Vec::with_capacity(20);
            let batch_size = 20; 

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
                            let rate = parts[1].parse().unwrap_or(0);
                        }
                    }
                    "AVAIL" => {
                        // ignore
                    }
                    "T" => {
                        if parts.len() == 2 {
                            let val: f32 = parts[1].parse().unwrap_or(0.0);
                            let reading = UnifiedReading::Thermo(
                                sensor_id.clone(),
                                ThermoReading {
                                    temperature_celsius: val,
                                },
                            );
                            local_buffer.push(reading);
                        }
                    }
                    "A" => {
                        if parts.len() == 4 {
                            let x: f32 = parts[1].parse().unwrap_or(0.0);
                            let y: f32 = parts[2].parse().unwrap_or(0.0);
                            let z: f32 = parts[3].parse().unwrap_or(0.0);
                            let reading = UnifiedReading::Accel(
                                sensor_id.clone(),
                                AccelReading {
                                    acceleration_x: x,
                                    acceleration_y: y,
                                    acceleration_z: z,
                                },
                            );
                            local_buffer.push(reading);
                        }
                    }
                    "F" => {
                        if parts.len() == 4 {
                            let x: f32 = parts[1].parse().unwrap_or(0.0);
                            let y: f32 = parts[2].parse().unwrap_or(0.0);
                            let z: f32 = parts[3].parse().unwrap_or(0.0);
                            let reading = UnifiedReading::Force(
                                sensor_id.clone(),
                                ForceReading {
                                    force_x: x,
                                    force_y: y,
                                    force_z: z,
                                },
                            );
                            local_buffer.push(reading);
                        }
                    }
                    _ => {}
                }

                if local_buffer.len() >= batch_size {
                    let batch = std::mem::take(&mut local_buffer);
                    manager.push_batch(batch);
                    local_buffer = Vec::with_capacity(batch_size);
                }
            }

            // 剩余数据
            if !local_buffer.is_empty() {
                manager.push_batch(local_buffer);
            }
        });
        self.threads.lock().unwrap().push(handle);
    }
///Push everything in the local buffer into main buffer at a time, obtain the lock once.
    pub fn push_batch(&self, items: Vec<UnifiedReading>) {
        let mut buf = self.buffer.lock().unwrap();
        let cur_cap = self.capacity.load(Ordering::Relaxed);

        for item in items {
                buf.push_back(item);
                self.total_writes.fetch_add(1, Ordering::Relaxed);
        }
        
        if buf.len() > cur_cap * 95 / 100 {
            self.overflow_warnings.fetch_add(1, Ordering::Relaxed);
            let count = self.threads.lock().unwrap().len();
            if count > 2 && count < 50 {
                self.capacity.store(cur_cap + 10000, Ordering::Relaxed);
            } else if count >= 50 {
                self.capacity.store(cur_cap + 50000, Ordering::Relaxed);
            }
        }
            self.not_empty.notify_all();
    }
    /// Prints statistics about the buffer (size, writes, reads, rates, warnings, threads).
    pub fn print_stats(&self) {
        let len = self.len();
        let capacity = self.capacity();
        let utilization = len as f64 / capacity as f64 * 100.0;
        let total_writes = self.total_writes.load(Ordering::Relaxed);
        let total_reads = self.total_reads.load(Ordering::Relaxed);
        let overflow = self.overflow_warnings.load(Ordering::Relaxed);
        let tcount = self.threads.lock().unwrap().len();
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let write_rate = total_writes as f64 / elapsed;
        let read_rate = total_reads as f64 / elapsed;

        println!(
            "Buffer Stats: size={}/{}, util={:.1}%, writes={}, reads={}, write_rate={:.0}/s, read_rate={:.0}/s, warnings={}, threads={}",
            len,
            capacity,
            utilization,
            total_writes,
            total_reads,
            write_rate,
            read_rate,
            overflow,
            tcount
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
        // lock is released, safely join
        for handle in handles {
            handle.join().ok();
        }
    }

    /// Returns `true` if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        let buf = self.buffer.lock().unwrap();
        buf.is_empty()
    }

    /// Push one piece of information for unit test
    pub fn push(&self, item: UnifiedReading) -> Result<(), UnifiedReading> {
        let mut buf = self.buffer.lock().unwrap();
        let current_cap = self.capacity.load(Ordering::Relaxed);
        if buf.len() >= current_cap {
            return Err(item);
        }
        buf.push_back(item);
        let current_len = buf.len();
        self.total_writes.fetch_add(1, Ordering::Relaxed);
        if current_len > current_cap * 95 / 100 {
            self.overflow_warnings.fetch_add(1, Ordering::Relaxed);
            let sensor_count = self.threads.lock().unwrap().len();
            const SENSOR_LESS: usize = 2;
            const SENSOR_MORE: usize = 50;
            if sensor_count > SENSOR_LESS&&sensor_count<SENSOR_MORE {
                let new_cap = current_cap + 10000;
                self.capacity.store(new_cap, Ordering::Relaxed);
            }else if sensor_count >= SENSOR_MORE {
                let new_cap = current_cap + 50000;
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

    /// Non-blocking pop: returns `Some(item)` if available, else `None`.
    pub fn try_pop(&self) -> Option<UnifiedReading> {
        let mut buf = self.buffer.lock().unwrap();
        if let Some(item) = buf.pop_front() {
            self.total_reads.fetch_add(1, Ordering::Relaxed);
            Some(item)
        } else {
            None
        }
    }
}
