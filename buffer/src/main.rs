use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Condvar, atomic::{AtomicUsize, Ordering}};
use std::fmt;
/// 单线程版的通用缓冲区管理器，底层用 VecDeque 做环形队列。
/// 这里用泛型 T，这样可以放任意类型的 SensorReading。
const CRITICAL: usize = 95;   // 马上要满
const WARNING: usize = 60;     // 需要注意
const SAFE: usize = 0;         
pub struct BufferManager<T> {
    buffer: Mutex<VecDeque<T>>,
    capacity: usize,
    not_empty: Condvar,
    total_writes: AtomicUsize,
    total_reads: AtomicUsize,
    peak_usage: AtomicUsize,
    overflow_warnings: AtomicUsize,
}

impl<T: Clone>  BufferManager<T> {
    /// 创建一个新的 BufferManager，指定容量上限。
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be > 0");
        Self {
            buffer: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            not_empty: Condvar::new(),
            total_writes: AtomicUsize::new(0),
            total_reads: AtomicUsize::new(0),
            peak_usage: AtomicUsize::new(0),
            overflow_warnings: AtomicUsize::new(0),
        }
    }

    /// 返回缓冲区容量（最大可存放的元素数）。
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 当前已使用的槽位数量。
    pub fn len(&self) -> usize {
        let buf = self.buffer.lock().unwrap();
        buf.len()
    }
    /// 缓冲区是否为空。
    pub fn is_empty(&self) -> bool {
        let buf = self.buffer.lock().unwrap();
        buf.is_empty()
    }

    /// 缓冲区是否已满。
    pub fn is_full(&self) -> bool {
        let buf = self.buffer.lock().unwrap();
        buf.len() >= self.capacity
    }


    /// 尝试向缓冲区写入一个元素。
    /// - 如果未满，push 成功，返回 Ok(())。

    pub fn push(&self, item: T) -> Result<(), T> {
       // 1. 获取锁
        let mut buf = self.buffer.lock().unwrap();

        if buf.len() >= self.capacity {
            return Err(item);  // 满了，返回 item 给调用者
        }
        
        // 3. 插入数据
        buf.push_back(item);
        
        // 4. 更新统计信息（在锁内完成，避免 race）
        let current_len = buf.len();
        self.total_writes.fetch_add(1, Ordering::Relaxed);
        
        // 更新峰值使用量
        let previous_peak = self.peak_usage.load(Ordering::Relaxed);
        if current_len > previous_peak {
            self.peak_usage.store(current_len, Ordering::Relaxed);
        }
        
        // 5. 检查是否接近满（使用当前长度，而不是重新获取）
        if current_len > self.capacity * 90 / 100 {
            self.overflow_warnings.fetch_add(1, Ordering::Relaxed);
            
            // 打印警告（但注意：这里还在锁内，打印可能稍慢）
            // 可以考虑移到锁外，但需要保存 current_len
            eprintln!("Warning：缓冲区使用率 {}% (容量: {})", 
                current_len * 100 / self.capacity,
                self.capacity
            );
        }
        self.not_empty.notify_one();
        Ok(())
    }
    
    /// 从缓冲区弹出一个元素（非阻塞）。
    ///
    /// - 如果有数据，返回 Some(item)。
    /// - 如果为空，返回 None。
   pub fn pop(&self) -> T {
        let mut buf = self.buffer.lock().unwrap();
        
        // 当缓冲区空时，阻塞等待
        while buf.is_empty() {
            buf = self.not_empty.wait(buf).unwrap();
        }
        
        let item = buf.pop_front().unwrap();
        self.total_reads.fetch_add(1, Ordering::Relaxed);
        
        item
    }
    /// “窥视”缓冲区头部元素，但不弹出。
    pub fn peek(&self) -> Option<T> {
        let buf = self.buffer.lock().unwrap();
        buf.front().cloned()  // 返回克隆的值
    }
}

