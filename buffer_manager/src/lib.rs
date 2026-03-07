use shared_global;
use std::collections::VecDeque;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
};
// use std::fmt;
use sensor_sim::traits::Sensor;
use shared_global::SensorType;
use shared_global::UnifiedReading;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::thread::{self, JoinHandle};
use std::time::Duration;
/// 单线程版的通用缓冲区管理器，底层用 VecDeque 做环形队列。
const CRITICAL: f64 = 0.2; // 马上要满
const WARNING: f64 = 0.25; // 需要注意
pub struct BufferManager<T> {
    buffer: Mutex<VecDeque<T>>,
    capacity: AtomicUsize,
    not_empty: Condvar,
    total_writes: AtomicUsize,
    total_reads: AtomicUsize,
    peak_usage: AtomicUsize,
    overflow_warnings: AtomicUsize,
    running: AtomicBool,                 // 全局运行标志
    threads: Mutex<Vec<JoinHandle<()>>>, // 所有读取线程句柄
    sensor_count: AtomicUsize,
}

impl BufferManager<UnifiedReading> {
    /// 创建一个新的 BufferManager，指定容量上限。
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be > 0");
        Self {
            buffer: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity: AtomicUsize::new(capacity),
            not_empty: Condvar::new(),
            total_writes: AtomicUsize::new(0),
            total_reads: AtomicUsize::new(0),
            peak_usage: AtomicUsize::new(0),
            overflow_warnings: AtomicUsize::new(0),
            running: AtomicBool::new(true),
            threads: Mutex::new(Vec::new()),
            sensor_count: AtomicUsize::new(0),
        }
    }

    /// 返回缓冲区容量（最大可存放的元素数）。
    pub fn capacity(&self) -> usize {
        self.capacity.load(Ordering::Relaxed)
    }

    /// 当前已使用的槽位数量。
    pub fn len(&self) -> usize {
        let buf = self.buffer.lock().unwrap();
        buf.len()
    }
    pub fn register_sensor(self: &Arc<Self>, mut sensor: SensorType, rate: u32) {
        self.sensor_count.fetch_add(1, Ordering::Relaxed);
        let manager = self.clone();
        let handle = thread::spawn(move || {
            while manager.running.load(Ordering::Relaxed) {
                // 获取当前传感器积压
                let available = match &sensor {
                    SensorType::Thermometer(t) => t.available(),
                    SensorType::Accelerometer(a) => a.available(),
                    SensorType::ForceSensor(f) => f.available(),
                };
                let emergency = (127 - available) as f64 / rate as f64; // 剩余时间

                // 根据紧急程度调整睡眠时间和本次最大读取量
                let (sleep_duration, max_batch) = if emergency < CRITICAL {
                    (Duration::from_micros(100), 40)
                } else if emergency < WARNING {
                    (Duration::from_millis(1), 10)
                } else {
                    (Duration::from_millis(5), 5)
                };

                // 先休眠（让出CPU），然后再读取
                thread::sleep(sleep_duration);

                // 确定本次实际要读取的数量
                let to_read = available.min(max_batch);
                if to_read > 0 {
                    let mut batch = Vec::with_capacity(to_read as usize);
                    for _ in 0..to_read {
                        let reading = match &sensor {
                            SensorType::Thermometer(t) => t.read().map(UnifiedReading::Thermo),
                            SensorType::Accelerometer(a) => a.read().map(UnifiedReading::Accel),
                            SensorType::ForceSensor(f) => f.read().map(UnifiedReading::Force),
                        };
                        if let Some(r) = reading {
                            batch.push(r);
                        }
                    }
                    // 批量推入缓冲区
                    if !batch.is_empty() {
                        let (accepted, rejected) = manager.push_batch(batch);
                        if !rejected.is_empty() {
                            // 缓冲区满了，被拒绝的数据可以稍后重试
                            thread::sleep(Duration::from_millis(1));
                            // 可选：将 rejected 重新放入下次批次
                        }
                    }
                } else {
                    // 无数据时短暂休眠
                    thread::sleep(Duration::from_millis(1));
                }
            }
            // 线程退出前，手动停止传感器内部线程
            match &mut sensor {
                SensorType::Thermometer(t) => t.stop(),
                SensorType::Accelerometer(a) => a.stop(),
                SensorType::ForceSensor(f) => f.stop(),
            }
        });

        // 将句柄存入线程列表
        self.threads.lock().unwrap().push(handle);
    }
    /// 优雅关闭所有读取线程
    pub fn shutdown(&self) {
        self.running.store(false, Ordering::Relaxed);

        // 先取出所有句柄，然后释放锁
        let handles = {
            let mut threads = self.threads.lock().unwrap();
            threads.drain(..).collect::<Vec<_>>()
        }; // 锁在这里自动释放

        // 此时锁已释放，可以安全地等待
        for handle in handles {
            thread::sleep(Duration::from_millis(1)); // 给线程一点退出时间
            if let Err(e) = handle.join() {
                eprintln!("线程 join 失败: {:?}", e);
            }
        }
    }
    /// 缓冲区是否为空。
    pub fn is_empty(&self) -> bool {
        let buf = self.buffer.lock().unwrap();
        buf.is_empty()
    }

    /// 缓冲区是否已满。
    pub fn is_full(&self) -> bool {
        let buf = self.buffer.lock().unwrap();
        buf.len() >= self.capacity.load(Ordering::Relaxed)
    }

    /// 尝试向缓冲区写入一个元素。
    /// - 如果未满，push 成功，返回 Ok(())。

    pub fn push(&self, item: UnifiedReading) -> Result<(), UnifiedReading> {
        // 1. 获取锁
        let mut buf = self.buffer.lock().unwrap();
        let current_cap = self.capacity.load(Ordering::Relaxed);
        if buf.len() >= current_cap {
            return Err(item);
        }
        buf.push_back(item);
        let current_len = buf.len();
        self.total_writes.fetch_add(1, Ordering::Relaxed);

        let previous_peak = self.peak_usage.load(Ordering::Relaxed);
        if current_len > previous_peak {
            self.peak_usage.store(current_len, Ordering::Relaxed);
        }

        // 检查使用率是否超过 90%
        if current_len > current_cap * 90 / 100 {
            self.overflow_warnings.fetch_add(1, Ordering::Relaxed);
            // eprintln!(
            //     "Warning：缓冲区使用率 {}% (容量: {})",
            //     current_len * 100 / current_cap,
            //     current_cap
            // );

            // 获取当前运行的传感器数量（从全局注册表）
            let sensor_count = self.sensor_count.load(Ordering::Relaxed);
            const SENSOR_THRESHOLD: usize = 3; // 阈值可调整
            if sensor_count > SENSOR_THRESHOLD {
                let new_cap = current_cap + 100000;
                self.capacity.store(new_cap, Ordering::Relaxed);
                // eprintln!("扩容：新容量 = {}", new_cap);
            }
        }
        self.not_empty.notify_one();
        Ok(())
    }
    pub fn push_batch(&self, items: Vec<UnifiedReading>) -> (usize, Vec<UnifiedReading>) {
        let mut buf = self.buffer.lock().unwrap(); // 只锁一次
        let current_cap = self.capacity.load(Ordering::Relaxed);
        let mut accepted = 0;
        let mut rejected = Vec::new();

        for item in items {
            if buf.len() >= current_cap {
                rejected.push(item);
            } else {
                buf.push_back(item);
                accepted += 1;
                self.total_writes.fetch_add(1, Ordering::Relaxed);
            }
        }

        // 更新统计信息
        let current_len = buf.len();
        let previous_peak = self.peak_usage.load(Ordering::Relaxed);
        if current_len > previous_peak {
            self.peak_usage.store(current_len, Ordering::Relaxed);
        }

        // 扩容检查（使用原子 sensor_count）
        if current_len > current_cap * 90 / 100 {
            self.overflow_warnings.fetch_add(1, Ordering::Relaxed);
            let sensor_count = self.sensor_count.load(Ordering::Relaxed);
            if sensor_count > 3 {
                let new_cap = current_cap + 100000;
                self.capacity.store(new_cap, Ordering::Relaxed);
            }
        }

        // 如果有数据写入，通知等待的读者
        if accepted > 0 {
            self.not_empty.notify_all(); // 可唤醒所有等待者
        }

        (accepted, rejected)
    }
    pub fn pop(&self) -> UnifiedReading {
        let mut buf = self.buffer.lock().unwrap();

        // 当缓冲区空时，阻塞等待
        while buf.is_empty() {
            buf = self.not_empty.wait(buf).unwrap();
        }

        let item = buf.pop_front().unwrap();
        self.total_reads.fetch_add(1, Ordering::Relaxed);

        item
    }
    pub fn pop_timeout(&self, timeout: Duration) -> Option<UnifiedReading> {
        let mut buf = self.buffer.lock().unwrap();
        if buf.is_empty() {
            let (new_buf, result) = self.not_empty.wait_timeout(buf, timeout).unwrap();
            buf = new_buf;
            if result.timed_out() {
                return None;
            }
        }
        let item = buf.pop_front().unwrap();
        self.total_reads.fetch_add(1, Ordering::Relaxed);
        Some(item)
    }
    pub fn try_pop(&self) -> Option<UnifiedReading> {
        let mut buf = self.buffer.lock().unwrap();
        buf.pop_front() // 非阻塞pop, 有数据就返回Some，没数据就返回None，
    }
    pub fn peek(&self) -> Option<UnifiedReading> {
        let buf = self.buffer.lock().unwrap();
        buf.front().cloned()
    }
}
