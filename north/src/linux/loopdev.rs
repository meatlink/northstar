// Copyright (c) 2019 - 2020 ESRLabs
//
//   Licensed under the Apache License, Version 2.0 (the "License");
//   you may not use this file except in compliance with the License.
//   You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//   Unless required by applicable law or agreed to in writing, software
//   distributed under the License is distributed on an "AS IS" BASIS,
//   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//   See the License for the specific language governing permissions and
//   limitations under the License.

use crate::SETTINGS;
use anyhow::{anyhow, Context, Result};
use async_std::{
    fs,
    path::{Path, PathBuf},
};
use libc::{c_int, ioctl};
use std::os::unix::prelude::*;

const LOOP_SET_FD: u16 = 0x4C00;
const LOOP_CLR_FD: u16 = 0x4C01;
const LOOP_SET_STATUS64: u16 = 0x4C04;
const LOOP_SET_DIRECT_IO: u16 = 0x4C08;
const LOOP_FLAG_READ_ONLY: u32 = 0x01;
const LOOP_FLAG_AUTOCLEAR: u32 = 0x04;
const LOOP_CTL_GET_FREE: u16 = 0x4C82;

#[derive(Debug)]
pub struct LoopControl {
    dev_file: fs::File,
}

impl LoopControl {
    pub async fn open() -> Result<LoopControl> {
        Ok(LoopControl {
            dev_file: fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&SETTINGS.devices.loop_control)
                .await?,
        })
    }

    pub async fn next_free(&self) -> Result<LoopDevice> {
        let result;
        unsafe {
            result = ioctl(self.dev_file.as_raw_fd() as c_int, LOOP_CTL_GET_FREE.into());
        }
        if result < 0 {
            Err(anyhow!(std::io::Error::last_os_error()))
        } else {
            Ok(LoopDevice::open(&format!("{}{}", SETTINGS.devices.loop_dev, result)).await?)
        }
    }
}

/// Interface to a loop device ie `/dev/loop0`.
#[derive(Debug)]
pub struct LoopDevice {
    device: fs::File,
    path: PathBuf,
}

impl AsRawFd for LoopDevice {
    fn as_raw_fd(&self) -> RawFd {
        self.device.as_raw_fd()
    }
}

impl LoopDevice {
    /// Opens a loop device.
    pub async fn open<P: AsRef<Path>>(dev: P) -> Result<LoopDevice> {
        let f = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(dev.as_ref())
            .await
            .context("Failed to open")?;
        Ok(LoopDevice {
            device: f,
            path: PathBuf::from(dev.as_ref()),
        })
    }

    pub fn attach_file(
        &self,
        bf_path: &Path,
        bf: &mut fs::File,
        offset: u64,
        sizelimit: u64,
        read_only: bool,
        auto_clear: bool,
    ) -> Result<()> {
        log::debug!(
            "Attaching {} to loopback device at {}",
            bf_path
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_else(|| bf_path.to_string_lossy()),
            self.path.to_string_lossy()
        );
        // Attach the file => Associate the loop device with the open file
        unsafe {
            if ioctl(
                self.device.as_raw_fd() as c_int,
                LOOP_SET_FD.into(),
                bf.as_raw_fd() as c_int,
            ) < 0
            {
                return Err(anyhow!("OS error: {}", std::io::Error::last_os_error()));
            }
        }

        // Set offset and limit for backing_file
        log::debug!("Setting offset {} and limit {}", offset, sizelimit);
        let mut info: loop_info64 = Default::default();
        info.lo_offset = offset;
        info.lo_sizelimit = sizelimit;
        if read_only {
            info.lo_flags |= LOOP_FLAG_READ_ONLY;
        }
        if auto_clear {
            info.lo_flags |= LOOP_FLAG_AUTOCLEAR;
        }

        unsafe {
            if ioctl(
                self.device.as_raw_fd() as c_int,
                LOOP_SET_STATUS64.into(),
                &mut info,
            ) < 0
            {
                self.detach()?;
                return Err(anyhow!("OS error: {}", std::io::Error::last_os_error()));
            }
        }
        Ok(())
    }

    pub fn detach(&self) -> Result<()> {
        unsafe {
            if ioctl(self.device.as_raw_fd() as c_int, LOOP_CLR_FD.into(), 0) < 0 {
                Err(anyhow!("OS error: {}", std::io::Error::last_os_error()))
            } else {
                Ok(())
            }
        }
    }

    pub fn set_direct_io(&self, enable: bool) -> Result<()> {
        unsafe {
            if ioctl(
                self.device.as_raw_fd() as c_int,
                LOOP_SET_DIRECT_IO.into(),
                if enable { 1 } else { 0 },
            ) < 0
            {
                Err(anyhow!("OS error: {}", std::io::Error::last_os_error()))
            } else {
                Ok(())
            }
        }
    }

    /// Get the path of the loop device.
    pub async fn path(&self) -> Option<PathBuf> {
        let mut p = PathBuf::from("/proc/self/fd");
        p.push(self.device.as_raw_fd().to_string());
        fs::read_link(&p).await.ok()
    }

    /// Get major and minor number of device
    pub async fn dev_id(&self) -> Result<(u64, u64)> {
        let attr = self.device.metadata().await?;
        let rdev = attr.rdev();
        let major = ((rdev >> 32) & 0xFFFF_F000) | ((rdev >> 8) & 0xFFF);
        let minor = ((rdev >> 12) & 0xFFFF_FF00) | (rdev & 0xFF);
        Ok((major, minor))
    }
}

#[repr(C)]
struct loop_info64 {
    pub lo_device: u64,
    pub lo_inode: u64,
    pub lo_rdevice: u64,
    pub lo_offset: u64,
    pub lo_sizelimit: u64,
    pub lo_number: u32,
    pub lo_encrypt_type: u32,
    pub lo_encrypt_key_size: u32,
    pub lo_flags: u32,
    pub lo_file_name: [u8; 64],
    pub lo_crypt_name: [u8; 64],
    pub lo_encrypt_key: [u8; 32],
    pub lo_init: [u64; 2],
}

impl Default for loop_info64 {
    fn default() -> loop_info64 {
        loop_info64 {
            lo_device: 0,
            lo_inode: 0,
            lo_rdevice: 0,
            lo_offset: 0,
            lo_sizelimit: 0,
            lo_number: 0,
            lo_encrypt_type: 0,
            lo_encrypt_key_size: 0,
            lo_flags: 0,
            lo_file_name: [0; 64],
            lo_crypt_name: [0; 64],
            lo_encrypt_key: [0; 32],
            lo_init: [0; 2],
        }
    }
}
