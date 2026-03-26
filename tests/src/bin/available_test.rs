use buffer_manager::BufferManager;
use rand::Rng;
use sensor_sim::accelerometer::Accelerometer;
use sensor_sim::force_sensor::ForceSensor;
use sensor_sim::thermometer::Thermometer;
use sensor_sim::traits::Sensor;
use shared_global::{ SensorType, UnifiedReading};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::process::{Command, Stdio,Child};
use std::sync::atomic::Ordering;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "--child" {
        // ---------- Child process mode: run sensor and output data ----------
        if args.len() < 5 {
            eprintln!("Child process: insufficient arguments");
            return;
        }
        let sensor_type: u32 = args[2].parse().unwrap();
        let id = args[3].clone();
        let rate: u32 = args[4].parse().unwrap();

        const ENABLE_ALGORITHM: bool = true; // Set to false to disable algorithm (AVAIL messages)
        // Create sensor
        let child_start = Instant::now();
        let enable_algorithm = ENABLE_ALGORITHM;
        match sensor_type {
            0 => {
                let mut thermo = Thermometer::new(id, rate);
                thermo.start();
                if enable_algorithm {
                    println!("RATE,{}", rate);
                }
                let mut last_avail_send = Instant::now();
                let mut read_count = 0;      // Number of readings taken from sensor
                let mut print_count = 0;     // Total lines output (including AVAIL)
                while child_start.elapsed() < Duration::from_secs(5) {
                    if let Some(reading) = thermo.read() {
                        read_count += 1;
                        println!("T,{}", reading.temperature_celsius);
                        print_count += 1;
                    }
                    if enable_algorithm && last_avail_send.elapsed() >= Duration::from_millis(100) {
                        let avail = thermo.available();
                        println!("AVAIL,{}", avail);
                        print_count += 1;
                        last_avail_send = Instant::now();
                    }
                    thread::sleep(Duration::from_micros(100));
                }
                //eprintln!("[Child temperature sensor] read {} items, output {} items", read_count, print_count);
            }
            1 => {
                let mut accel = Accelerometer::new(id, rate);
                accel.start();
                if enable_algorithm {
                    println!("RATE,{}", rate);
                }
                let mut last_avail_send = Instant::now();
                let mut read_count = 0;
                let mut print_count = 0;
                while child_start.elapsed() < Duration::from_secs(5) {
                    if let Some(reading) = accel.read() {
                        read_count += 1;
                        println!("A,{},{},{}", reading.acceleration_x, reading.acceleration_y, reading.acceleration_z);
                        print_count += 1;
                    }
                    if enable_algorithm && last_avail_send.elapsed() >= Duration::from_millis(100) {
                        let avail = accel.available();
                        println!("AVAIL,{}", avail);
                        print_count += 1;
                        last_avail_send = Instant::now();
                    }
                    thread::sleep(Duration::from_micros(100));
                }
                // eprintln!("[Child accelerometer] read {} items, output {} items", read_count, print_count);
            }
            2 => {
                let mut force = ForceSensor::new(id, rate);
                force.start();
                if enable_algorithm {
                    println!("RATE,{}", rate);
                }
                let mut last_avail_send = Instant::now();
                let mut read_count = 0;
                let mut print_count = 0;
                while child_start.elapsed() < Duration::from_secs(5) {
                    if let Some(reading) = force.read() {
                        read_count += 1;
                        println!("F,{},{},{}", reading.force_x, reading.force_y, reading.force_z);
                        print_count += 1;
                    }
                    if enable_algorithm && last_avail_send.elapsed() >= Duration::from_millis(100) {
                        let avail = force.available();
                        println!("AVAIL,{}", avail);
                        print_count += 1;
                        last_avail_send = Instant::now();
                    }
                    thread::sleep(Duration::from_micros(100));
                }
                // eprintln!("[Child force sensor] read {} items, output {} items", read_count, print_count);
            }
            _ => {}
        }
        return;
    }

    // ---------- Parent process mode: start multiple child processes and read data ----------
    let buffer = Arc::new(BufferManager::<UnifiedReading>::new(30000));
    let start_time = Instant::now();

    /// Spawn child processes
    let mut children: Vec<Child> = vec![];
    for i in 0..150 {
        let sensor_type = rand::thread_rng().gen_range(0..3);
        let rate = if i < 1000 { 1000 } else { 100 };
        let id = format!("sensor-{}", i);
        let exe = std::env::current_exe().unwrap();

        let mut child = Command::new(exe)
            .arg("--child")
            .arg(sensor_type.to_string())
            .arg(id.clone())
            .arg(rate.to_string())
            .stdout(Stdio::piped())
            .spawn()
            .expect("Failed to start child process");

        let stdout = child.stdout.take().unwrap();
         // buffer.register_pipe_reader(id, stdout);
        buffer.register_pipe_reader_no_edf(id, stdout);
        children.push(child);
    }
    let ready_time = Instant::now();
    println!("Child process startup time: {:?}", ready_time.duration_since(start_time));
    let writes_before = buffer.total_writes.load(Ordering::Relaxed);

    // Run for 5 seconds
    let run_start = Instant::now();
    thread::sleep(Duration::from_secs(5));
    let run_end = Instant::now();
    let run_duration = run_end.duration_since(run_start);
    println!("Actual run duration: {:?} (expected 5s)", run_duration);
    let writes_after = buffer.total_writes.load(Ordering::Relaxed);
    let writes_in_run = writes_after - writes_before;
    let run_rate = writes_in_run as f64 / run_duration.as_secs_f64();
    println!("Writes during run: {}, rate: {:.0}/s", writes_in_run, run_rate);

    // Wait for child processes to finish and shut down
    thread::sleep(Duration::from_millis(500));
    for mut child in children {
        let _ = child.wait();
    }
    let shutdown_end = Instant::now();
    println!("Total elapsed: {:?}", shutdown_end.duration_since(start_time));
    println!("Buffer size: {}", buffer.len());
    // Print statistics
    buffer.print_stats();
    println!("Parent process finished, buffer size: {}", buffer.len());
}
