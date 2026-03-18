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
use std::process::{Command, Stdio,Child};
use std::io::{BufRead, BufReader};
use std::sync::atomic::Ordering;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "--child" {
        // ---------- 子进程模式：运行传感器并输出数据 ----------
        if args.len() < 5 {
            eprintln!("子进程参数不足");
            return;
        }
        let sensor_type: u32 = args[2].parse().unwrap();
        let id = args[3].clone();
        let rate: u32 = args[4].parse().unwrap();

        // 先发送一次速率信息
        //println!("RATE,{}", rate);
        const ENABLE_ALGORITHM: bool = true; // 改为 false 即可关闭算法
        // 创建传感器
        let child_start = Instant::now();
        //对比试验
        let enable_algorithm =ENABLE_ALGORITHM;
        match sensor_type {
            0 => {
                let mut thermo = Thermometer::new(id, rate);
                thermo.start();
                if enable_algorithm {
                    println!("RATE,{}", rate);
                }
                let mut last_avail_send = Instant::now();
                while child_start.elapsed() < Duration::from_secs(5) {
                    if let Some(reading) = thermo.read() {
                        println!("T,{}", reading.temperature_celsius);
                    }
                    if enable_algorithm && last_avail_send.elapsed() >= Duration::from_millis(100) {
                        let avail = thermo.available();
                        println!("AVAIL,{}", avail);
                        last_avail_send = Instant::now();
                    }
                    thread::sleep(Duration::from_micros(100));
                }
            }
            1 => {
                let mut accel = Accelerometer::new(id, rate);
                accel.start();
                if enable_algorithm {
                    println!("RATE,{}", rate);
                }
                let mut last_avail_send = Instant::now();
                let mut count = 0;
                while child_start.elapsed() < Duration::from_secs(5) {
                    if let Some(reading) = accel.read() {
                        println!("A,{},{},{}", reading.acceleration_x, reading.acceleration_y, reading.acceleration_z);
                    count+=1
                    }
                    if enable_algorithm && last_avail_send.elapsed() >= Duration::from_millis(100) {
                        let avail = accel.available();
                        println!("AVAIL,{}", avail);
                        last_avail_send = Instant::now();
                    }
                }
                eprintln!("子进程  读取了 {} 条数据", count);
            }
            2 => {
                let mut force = ForceSensor::new(id, rate);
                force.start();
                if enable_algorithm {
                    println!("RATE,{}", rate);
                }
                let mut last_avail_send = Instant::now();
                while child_start.elapsed() < Duration::from_secs(5) {
                    if let Some(reading) = force.read() {
                        println!("F,{},{},{}", reading.force_x, reading.force_y, reading.force_z);
                    }
                    if enable_algorithm && last_avail_send.elapsed() >= Duration::from_millis(100) {
                        let avail = force.available();
                        println!("AVAIL,{}", avail);
                        last_avail_send = Instant::now();
                    }
                }
            }
            _ => panic!("未知传感器类型"),
        }
        // match sensor_type {
        //     0 => {
        //         let mut thermo = Thermometer::new(id, rate);
        //         thermo.start();
        //         let mut last_avail_send = Instant::now();
        //         while child_start.elapsed() < Duration::from_secs(5) {
        //             if let Some(reading) = thermo.read() {
        //                 println!("T,{}",  reading.temperature_celsius);
        //             }
        //             // 每100ms发送一次available
        //             if last_avail_send.elapsed() >= Duration::from_millis(100) {
        //                 let avail = thermo.available();
        //                 println!("AVAIL,{}", avail);
        //                 last_avail_send = Instant::now();
        //             }
        //             }
        //         }
        //
        //     1 => {
        //         let mut accel = Accelerometer::new(id, rate);
        //         accel.start();
        //         let mut last_avail_send = Instant::now();
        //         while child_start.elapsed() < Duration::from_secs(5) {
        //             if let Some(reading) = accel.read() {
        //                 println!("A,{},{},{}", reading.acceleration_x, reading.acceleration_y, reading.acceleration_z);
        //             }
        //             if last_avail_send.elapsed() >= Duration::from_millis(100) {
        //                 let avail = accel.available();
        //                 println!("AVAIL,{}", avail);
        //                 last_avail_send = Instant::now();
        //             }
        //
        //         }
        //     }
        //     2 => {
        //         let mut force = ForceSensor::new(id, rate);
        //         force.start();
        //         let mut last_avail_send = Instant::now();
        //         while child_start.elapsed() < Duration::from_secs(5) {
        //             if let Some(reading) = force.read() {
        //                 println!("F,{},{},{}", reading.force_x, reading.force_y, reading.force_z);
        //             }
        //             if last_avail_send.elapsed() >= Duration::from_millis(100) {
        //                 let avail = force.available();
        //                 println!("AVAIL,{}", avail);
        //                 last_avail_send = Instant::now();
        //             }
        //
        //         }
        //     }
        //     _ => panic!("未知传感器类型"),
        // }
        return;
    }

    // ---------- 父进程模式：启动多个子进程并读取数据 ----------
    let buffer = Arc::new(BufferManager::<UnifiedReading>::new(30000));
    let start_time = Instant::now();

    // 启动子进程
    let mut children: Vec<Child> = vec![];
    for i in 0..100 { // 启动100个子进程
        let sensor_type = rand::thread_rng().gen_range(0..3);
        let rate = 100;
        let id = format!("sensor-{}", i);
        let exe = std::env::current_exe().unwrap();

        let mut child = Command::new(exe)
            .arg("--child")
            .arg(sensor_type.to_string())
            .arg(id.clone())
            .arg(rate.to_string())
            .stdout(Stdio::piped())
            .spawn()
            .expect("启动子进程失败");

        let stdout = child.stdout.take().unwrap();
        buffer.register_pipe_reader(id, stdout);
        children.push(child);
    }
    let ready_time = Instant::now();
    println!("子进程启动耗时: {:?}", ready_time.duration_since(start_time));
    let writes_before = buffer.total_writes.load(Ordering::Relaxed);

    // 正式运行5秒
    let run_start = Instant::now();
    thread::sleep(Duration::from_secs(5));
    let run_end = Instant::now();
    let run_duration = run_end.duration_since(run_start);
    println!("实际运行时间: {:?} (期望5秒)", run_duration);
    // 记录运行开始前的写入数
    // 记录运行结束后的写入数
    let writes_after = buffer.total_writes.load(Ordering::Relaxed);
    let writes_in_run = writes_after - writes_before;
    let run_rate = writes_in_run as f64 / run_duration.as_secs_f64();
    println!("运行期间写入数: {}, 速率: {:.0}/s", writes_in_run, run_rate);

    // 等待子进程和关闭（原代码）
    thread::sleep(Duration::from_millis(500));
    for mut child in children {
        let _ = child.wait();
    }
    let shutdown_end = Instant::now();
    println!("总耗时: {:?}", shutdown_end.duration_since(start_time));

    // 打印统计信息
    buffer.print_stats();
    println!("父进程结束，缓冲区大小: {}", buffer.len());
}