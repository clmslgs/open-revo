//! Read the Intel package digital thermal sensor through an installed PawnIO driver.
//! ACPI thermal zones are intentionally excluded: they can describe the chassis.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::time::{Duration, Instant};

const INTEL_MSR_MODULE: &[u8] = include_bytes!("../../resources/pawnio/IntelMSR.bin");
const IOCTL_LOAD_BINARY: u32 = (41394 << 16) | (0x821 << 2);
const IOCTL_EXECUTE: u32 = (41394 << 16) | (0x841 << 2);
const MSR_TEMPERATURE_TARGET: u64 = 0x1A2;
const MSR_PACKAGE_THERM_STATUS: u64 = 0x1B1;
const TEMPERATURE_VALID: u64 = 1 << 31;
const PROBE_INTERVAL: Duration = Duration::from_secs(30);

#[link(name = "kernel32")]
extern "system" {
    fn DeviceIoControl(
        device: isize,
        control_code: u32,
        input: *const std::ffi::c_void,
        input_size: u32,
        output: *mut std::ffi::c_void,
        output_size: u32,
        returned: *mut u32,
        overlapped: *mut std::ffi::c_void,
    ) -> i32;
    fn GetCurrentThread() -> *mut std::ffi::c_void;
    fn SetThreadAffinityMask(thread: *mut std::ffi::c_void, mask: usize) -> usize;
}

fn supports_package_sensor() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::__cpuid;
        let vendor = __cpuid(0);
        return vendor.eax >= 6
            && vendor.ebx == u32::from_le_bytes(*b"Genu")
            && vendor.edx == u32::from_le_bytes(*b"ineI")
            && vendor.ecx == u32::from_le_bytes(*b"ntel")
            && (__cpuid(6).eax & (1 << 6)) != 0;
    }
    #[cfg(not(target_arch = "x86_64"))]
    false
}

fn decode_package_temperature(target: u64, status: u64) -> Option<i32> {
    if status & TEMPERATURE_VALID == 0 {
        return None;
    }
    // Read the hardware TjMax, independently of configurable temperature walls.
    let tj_max = ((target >> 16) & 0xFF) as i32;
    if !(70..=125).contains(&tj_max) {
        return None;
    }
    let delta = ((status >> 16) & 0x7F) as i32;
    let celsius = tj_max - delta;
    (celsius > 0 && celsius <= tj_max).then_some(celsius)
}

struct ThreadAffinity(usize);

impl ThreadAffinity {
    fn pin() -> io::Result<Self> {
        // Both MSRs must be read on the same CPU/package.
        let previous = unsafe { SetThreadAffinityMask(GetCurrentThread(), 1) };
        if previous == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(previous))
    }
}

impl Drop for ThreadAffinity {
    fn drop(&mut self) {
        unsafe {
            SetThreadAffinityMask(GetCurrentThread(), self.0);
        }
    }
}

/// A handle scoped to the signed IntelMSR module; this API only reads temperature.
pub struct IntelPackageTemperature {
    device: File,
}

impl IntelPackageTemperature {
    /// Connect to an already installed driver. This never installs or starts services.
    pub fn open() -> io::Result<Self> {
        if !supports_package_sensor() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Intel package thermal sensor unavailable",
            ));
        }
        let sensor = Self {
            device: OpenOptions::new()
                .read(true)
                .write(true)
                .access_mode(3)
                .share_mode(3)
                .open(r"\\?\GLOBALROOT\Device\PawnIO")?,
        };
        sensor.ioctl(IOCTL_LOAD_BINARY, INTEL_MSR_MODULE, &mut [])?;
        Ok(sensor)
    }

    fn ioctl(&self, code: u32, input: &[u8], output: &mut [u8]) -> io::Result<usize> {
        let mut returned = 0;
        let success = unsafe {
            DeviceIoControl(
                self.device.as_raw_handle() as isize,
                code,
                input.as_ptr().cast(),
                input.len() as u32,
                if output.is_empty() {
                    std::ptr::null_mut()
                } else {
                    output.as_mut_ptr().cast()
                },
                output.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(returned as usize)
    }

    fn read_msr(&self, index: u64) -> io::Result<u64> {
        // PawnIO ABI: 32-byte, NUL-padded function name followed by 64-bit cells.
        let mut input = [0u8; 40];
        let operation = b"ioctl_read_msr";
        input[..operation.len()].copy_from_slice(operation);
        input[32..].copy_from_slice(&index.to_le_bytes());
        let mut output = [0u8; 8];
        if self.ioctl(IOCTL_EXECUTE, &input, &mut output)? != output.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Incomplete temperature MSR read",
            ));
        }
        Ok(u64::from_le_bytes(output))
    }

    pub fn read_celsius(&self) -> io::Result<i32> {
        let _affinity = ThreadAffinity::pin()?;
        let target = self.read_msr(MSR_TEMPERATURE_TARGET)?;
        let status = self.read_msr(MSR_PACKAGE_THERM_STATUS)?;
        decode_package_temperature(target, status).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid CPU package temperature sample",
            )
        })
    }
}

/// Reconnect periodically so installing PawnIO does not require restarting OpenRevo.
pub struct PackageTemperatureReader {
    sensor: Option<IntelPackageTemperature>,
    next_probe: Instant,
    supported: bool,
    reported_unavailable: bool,
}

impl PackageTemperatureReader {
    pub fn new() -> Self {
        Self {
            sensor: None,
            next_probe: Instant::now(),
            supported: supports_package_sensor(),
            reported_unavailable: false,
        }
    }

    pub fn read_celsius(&mut self) -> Option<i32> {
        if !self.supported {
            return None;
        }
        if self.sensor.is_none() {
            if Instant::now() < self.next_probe {
                return None;
            }
            self.next_probe = Instant::now() + PROBE_INTERVAL;
            match IntelPackageTemperature::open() {
                Ok(sensor) => {
                    crate::log_info!("CPU_TEMP", "CPU Package sensor connected through PawnIO");
                    self.sensor = Some(sensor);
                    self.reported_unavailable = false;
                }
                Err(error) => {
                    if !self.reported_unavailable {
                        crate::log_warn!("CPU_TEMP", "CPU Package sensor unavailable: {}. Signed PawnIO driver: https://pawnio.eu", error);
                        self.reported_unavailable = true;
                    }
                    return None;
                }
            }
        }
        match self.sensor.as_ref()?.read_celsius() {
            Ok(celsius) => Some(celsius),
            Err(error) => {
                crate::log_warn!("CPU_TEMP", "CPU Package sample unavailable: {}", error);
                self.sensor = None;
                self.next_probe = Instant::now() + PROBE_INTERVAL;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_hardware_tjmax_for_package_temperature() {
        for (tj_max, delta) in [(100, 57), (105, 62), (110, 67)] {
            assert_eq!(
                decode_package_temperature(tj_max << 16, TEMPERATURE_VALID | (delta << 16)),
                Some(43)
            );
        }
    }

    #[test]
    fn accepts_tjmax_without_clamping_overheating_readings() {
        assert_eq!(
            decode_package_temperature(110 << 16, TEMPERATURE_VALID),
            Some(110)
        );
    }

    #[test]
    fn rejects_invalid_samples_instead_of_manufacturing_a_temperature() {
        assert_eq!(decode_package_temperature(100 << 16, 57 << 16), None);
        assert_eq!(decode_package_temperature(0, TEMPERATURE_VALID), None);
        assert_eq!(
            decode_package_temperature(255 << 16, TEMPERATURE_VALID),
            None
        );
        assert_eq!(
            decode_package_temperature(100 << 16, TEMPERATURE_VALID | (110 << 16)),
            None
        );
    }
}
