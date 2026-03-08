use buffer_manager::BufferManager;
use rand::Rng;
use sensor_sim::accelerometer::Accelerometer;
use sensor_sim::force_sensor::ForceSensor;
use sensor_sim::thermometer::Thermometer;
use sensor_sim::traits::Sensor;
use shared_global::{ SensorType, UnifiedReading};
use std::net::Shutdown;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;
fn main() {
    let mut rng = rand::thread_rng();
    let buffer = Arc::new(BufferManager::<UnifiedReading>::new(30000));
    // 1. 创建、启动并注册传感器
    let start_time = Instant::now();
    const SIZE: i32 = 550;
    let mut rate = 1000;
    for i in 0..SIZE {
        let sensor_type = rng.gen_range(0..3);
        // rate = rng.gen_range(100..1001);
        let id = format!("sensor-{}", i);

        match sensor_type {
            0 => {
                let mut thermo = Thermometer::new(id.clone(), rate);
                thermo.start();
                buffer.register_sensor(SensorType::Thermometer(thermo), rate);
            }
            1 => {
                let mut accel = Accelerometer::new(id.clone(), rate);
                accel.start();
                buffer.register_sensor(SensorType::Accelerometer(accel), rate);
            }
            2 => {
                let mut force = ForceSensor::new(id.clone(), rate);
                force.start();
                buffer.register_sensor(SensorType::ForceSensor(force), rate);
            }
            _ => unreachable!(),
        }
    }
    let mut elapsed = start_time.elapsed();
    println!("创建sensor用时: {:?}", elapsed);
    // 4. 主线程监控
    let buffer_clone = buffer.clone();

    // let sampler = thread::spawn(move || {
    //     let mut sampled = 0;
    //     let max_samples = 3;
    //     while sampled < max_samples {
    //         thread::sleep(Duration::from_secs(2)); // 每2秒抽样一次
    //         // 尝试弹出数据（非阻塞）
    //         match buffer_clone.try_pop() {
    //             Some(reading) => {
    //                 sampled += 1;
    //                 // 根据类型验证数据有效性
    //                 match reading {
    //                     UnifiedReading::Thermo(t) => {
    //                         println!("✅ 抽样温度: {:.2}°C", t.temperature_celsius);
    //                         // 验证温度范围 (20-30 是合理的随机范围)
    //                         assert!(t.temperature_celsius >= 20.0 && t.temperature_celsius <= 30.0);
    //                     }
    //                     UnifiedReading::Accel(a) => {
    //                         println!("✅ 抽样加速度: x={:.2}, y={:.2}, z={:.2}",
    //                             a.acceleration_x, a.acceleration_y, a.acceleration_z);
    //                         // 加速度范围 ±5
    //                         assert!(a.acceleration_x.abs() <= 5.0);
    //                         assert!(a.acceleration_y.abs() <= 5.0);
    //                         assert!(a.acceleration_z.abs() <= 5.0);
    //                     }
    //                     UnifiedReading::Force(f) => {
    //                         println!("✅ 抽样力: x={:.2}, y={:.2}, z={:.2}",
    //                             f.force_x, f.force_y, f.force_z);
    //                         // 力范围 0-100
    //                         assert!(f.force_x >= 0.0 && f.force_x <= 100.0);
    //                         assert!(f.force_y >= 0.0 && f.force_y <= 100.0);
    //                         assert!(f.force_z >= 0.0 && f.force_z <= 100.0);
    //                     }
    //                 }
    //             }
    //             None => {
    //                 println!("⏳ 缓冲区暂无数据，等待...");
    //             }
    //         }
    //     }
    //     println!("📊 抽样完成，共检查 {} 个样本", max_samples);
    // });

    // 5. 主线程监控
    // 运行一段时间后退出（避免无限运行）
    let monitor_duration = Duration::from_secs(5);
    let start = std::time::Instant::now();

    while start.elapsed() < monitor_duration {
        thread::sleep(Duration::from_secs(1));
        println!("缓冲区大小: {}", buffer.len());
    }
    elapsed = start_time.elapsed();
    println!("shutdown前实际运行时间: {:?}", elapsed);
    buffer.print_stats();
    let shutdown = Instant::now();
    buffer.shutdown();
    println!("缓冲区大小: {}", buffer.len());
    // 等待抽样线程完成（它应该已经在 max_samples 后结束）
    // sampler.join().unwrap();
    elapsed = start_time.elapsed();
    println!("实际运行总时间: {:?}", elapsed);
    let mut t1=shutdown.elapsed();
    println!("关闭耗时：{:?}",t1);
    println!("✅ 测试完成");
}
