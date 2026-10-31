//! Real-machine checks. Default is read-only; --verify-controls performs bounded reversible writes.
use openrevo_core::hal::ptl_pmc as ptl;
use std::{thread, time::Duration};

struct Restore(u8);
impl Drop for Restore {
    fn drop(&mut self) {
        eprintln!("Restore fan: {:?}", ptl::restore_auto());
        eprintln!("Restore battery: {:?}", ptl::set_battery_limit(self.0));
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|a| a == "--sample-fans") {
        let mut sensor = openrevo_core::hal::cpu_temperature::PackageTemperatureReader::new();
        for sample in 1..=8 {
            println!("Sample {sample}: CPU Package {:?} C; fan RPM {:?}", sensor.read_celsius(), ptl::read_rpm()?);
            if sample < 8 { thread::sleep(Duration::from_secs(1)); }
        }
        return Ok(());
    }
    let battery = ptl::battery_state()?;
    println!("Battery: {:?}; RPM: {:?}", battery, ptl::read_rpm()?);
    if !std::env::args().any(|a| a == "--verify-controls") {
        return Ok(());
    }
    let original_limit = if battery.enabled { battery.lower } else { 100 };
    if ![60, 80, 100].contains(&original_limit) {
        return Err("Cannot restore this battery threshold with this probe".into());
    }
    let _restore = Restore(original_limit);
    let verify_low = std::env::args().any(|a| a == "--verify-low-speed");
    if verify_low {
        ptl::set_manual_rpm([2000, 2000])?;
        thread::sleep(Duration::from_secs(8));
        let low = ptl::read_rpm()?;
        println!("Low-speed startup 2000/2000 => {low:?}");
        if low.iter().any(|rpm| rpm.abs_diff(2000) > 900) {
            return Err("Low-speed startup did not converge".into());
        }
    }
    ptl::set_manual_rpm([4000, 4500])?;
    thread::sleep(Duration::from_secs(5));
    let rpm = ptl::read_rpm()?;
    let display = openrevo_core::hal::get_hardware_driver().read_fan_rpm()?;
    println!("HAL display RPM => {display:?}");
    if display.0.abs_diff(rpm[0]) > 900 || display.1.abs_diff(rpm[1]) > 900 {
        return Err("HAL display does not agree with private fan tachometers".into());
    }
    println!("Manual 4000/4500 => {:?}; {:?}", rpm, ptl::status());
    if rpm[0].abs_diff(4000) > 900 || rpm[1].abs_diff(4500) > 900 {
        return Err("Manual fan RPM did not converge".into());
    }
    ptl::set_curve(vec![(30, 50), (100, 50)])?;
    thread::sleep(Duration::from_secs(4));
    println!(
        "Curve PWM 50 => {:?}; {:?}",
        ptl::read_rpm()?,
        ptl::status()
    );
    assert_eq!(ptl::status().mode, "curve");
    assert_eq!(ptl::status().duty_percent, Some(50));
    if verify_low {
        ptl::set_curve(vec![(30, 25), (100, 25)])?;
        thread::sleep(Duration::from_secs(5));
        let low = ptl::read_rpm()?;
        println!("Minimum curve PWM25 => {low:?}");
        if low.contains(&0) {
            return Err("Minimum PWM did not keep both fans spinning".into());
        }
    }
    ptl::suspend_or_exit();
    assert_eq!(ptl::status().mode, "auto");
    for limit in [60, 80, 100] {
        ptl::set_battery_limit(limit)?;
        let read = ptl::battery_state()?;
        println!("Battery {limit}% => {read:?}");
        assert_eq!(read.enabled, limit != 100);
        if read.enabled {
            assert_eq!(read.lower, limit);
            assert_eq!(read.upper, 100);
        }
    }
    Ok(())
}
