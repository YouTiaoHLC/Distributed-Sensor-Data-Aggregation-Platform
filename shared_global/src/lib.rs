#[macro_use]
extern crate lazy_static;

// 2. 导入需要的类型
use sensor_sim::thermometer::{Thermometer, ThermoReading};
use sensor_sim::accelerometer::{Accelerometer, AccelReading};
use sensor_sim::force_sensor::{ForceSensor, ForceReading};
use sensor_sim::traits::Sensor;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub enum SensorType {
    Thermometer(Thermometer),
    Accelerometer(Accelerometer),
    ForceSensor(ForceSensor),
}
#[derive(Clone)]
pub enum UnifiedReading {
    Thermo(ThermoReading),   // 温度计读数
    Accel(AccelReading),      // 加速度计读数
    Force(ForceReading),      // 力传感器读数
}

// 2. 传感器信息 - 记录在注册表里
pub struct SensorInfo {
    pub sensor: SensorType,  // 存这个 enum
    pub rate: u32,
}

// 3. 全局注册表 - HashMap<String, SensorInfo>
lazy_static! {
    pub static ref SENSOR_REGISTRY: Arc<RwLock<HashMap<String, SensorInfo>>> = 
        Arc::new(RwLock::new(HashMap::new()));
}

// 4. 注册函数 - 传入 enum，自动记录类型
pub fn register_sensor(sensor: SensorType, rate: u32) -> String {
    let id = match &sensor {
        SensorType::Thermometer(t) => t.id(),
        SensorType::Accelerometer(a) => a.id(),
        SensorType::ForceSensor(f) => f.id(),
    };
    
    let info = SensorInfo { sensor, rate };
    SENSOR_REGISTRY.write().unwrap().insert(id.clone(), info);
    id
}

// 5. 获取传感器数量 
pub fn get_sensor_count() -> usize {
    SENSOR_REGISTRY.read().unwrap().len()
}