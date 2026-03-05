use std::env;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Condvar, atomic::{AtomicUsize, Ordering}};
use std::fmt;

fn main() {
    // 从命令行读取参数
    let args: Vec<String> = env::args().collect();
    let sensor_count = if args.len() > 1 {
        args[1].parse().unwrap_or(5)  // 默认5个
    } else {
        5
    };
    
    println!("启动 {} 个传感器", sensor_count);
    
    let mut sensors = vec![];
    
    for i in 0..sensor_count {
        // 轮流创建不同类型的传感器
        let sensor_type = i % 3;
        let rate = match sensor_type {
            0 => 10,  // 温度计
            1 => 20,  // 加速度计
            2 => 15,  // 力传感器
            _ => 10,
        };
        
        let id = format!("sensor-{}-{}", 
            match sensor_type {
                0 => "thermo",
                1 => "accel",
                2 => "force",
                _ => "unknown",
            },
            i
        );
        
        let sensor: Box<dyn Sensor> = match sensor_type {
            0 => Box::new(Thermometer::new(id, rate)),
            1 => Box::new(Accelerometer::new(id, rate)),
            2 => Box::new(ForceSensor::new(id, rate)),
            _ => unreachable!(),
        };
        
        sensors.push(sensor);
    }
    
    // 启动传感器...
    for sensor in &mut sensors {
        sensor.start();
    }
    // 为每个传感器创建读取线程
    let buffer = Arc::new(BufferManager::new(1000));
    let mut handles = vec![];
    
    for sensor in sensors {
        let bm = buffer.clone();
        let handle = thread::spawn(move || {
            // 每个传感器一个线程
            loop {
                while let Some(reading) = sensor.read() {
                    bm.push(reading.into());  // 转换成统一类型
                }
                thread::sleep(Duration::from_micros(500));
            }
        });
        handles.push(handle);
    }
}