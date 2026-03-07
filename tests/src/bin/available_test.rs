use sensor_sim::thermometer::Thermometer;
use sensor_sim::traits::Sensor;
use std::time::{Duration, Instant};
use std::thread;

fn main() {
    println!("=== 修正版传感器行为测试 ===\n");
    
    let mut sensor = Thermometer::new("test".to_string(), 20);  // 20条/秒
    sensor.start();
    
    // 等传感器热身
    thread::sleep(Duration::from_millis(200));
    
    // 场景1：正确的"连续读取"测试
    println!("[场景1] 连续读取（每秒读10次，每次读完所有积压）");
    let start = Instant::now();
    let mut total_read = 0;
    let mut max_available = 0;
    
    while start.elapsed() < Duration::from_secs(1) {
        let before = sensor.available();
        if before > max_available { max_available = before; }
        
        // ✅ 修正：一次读完所有可读的数据
        let mut read_count = 0;
        while let Some(r) = sensor.read() {
            read_count += 1;
            total_read += 1;
            // if read_count == 1 {
                println!("  读前 available: {:2}, 一次读走 {} 条, 最新值: {:.2}°C", 
                    before, read_count, r.temperature_celsius);
            // }
        }
        
        println!("    读后 available: {:2}", sensor.available());
        thread::sleep(Duration::from_millis(100));
    }
    println!("  1秒内共读 {} 条 (理论20条)", total_read);
    println!("  最大积压: {} 条", max_available);
    
    // 清空积压
    while sensor.read().is_some() {}
    
    // 场景3：读取速度 vs 生产速度
    println!("\n[场景3] 不同读取频率对比");
    
    for &sleep_ms in &[200, 100, 50, 20, 10] {
        println!("\n  读取频率: 每 {}ms 读一次", sleep_ms);
        
        thread::sleep(Duration::from_millis(500));  // 让系统稳定
        
        let start = Instant::now();
        let mut peak = 0;
        let mut total = 0;
        
        while start.elapsed() < Duration::from_secs(2) {
            // 读取所有积压
            let before = sensor.available();
            if before > peak { peak = before; }
            
            let mut count = 0;
            while let Some(_) = sensor.read() {
                count += 1;
            }
            total += count;
            
            thread::sleep(Duration::from_millis(sleep_ms));
        }
        
        println!("    平均每秒读 {} 条", total / 2);
        println!("    最大积压 {} 条", peak);
    }
    
    sensor.stop();
}