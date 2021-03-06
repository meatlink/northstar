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

use anyhow::Result;

pub mod cgroups;
pub mod device_mapper;
pub mod loopdev;
pub mod mount;

pub async fn init() -> Result<()> {
    use crate::SETTINGS;
    use anyhow::Context;

    log::debug!("Entering mount namespace");
    let r = unsafe { libc::unshare(libc::CLONE_NEWNS) };
    if r != 0 {
        return Err(anyhow::anyhow!(
            "Failed to unshare: {}",
            std::io::Error::last_os_error()
        ));
    }

    // Set mount propagation to PRIVATE on /data
    // Mounting with MS_PRIVATE fails on Android on
    // a non private tree.
    mount::mount(
        &SETTINGS.devices.unshare_root,
        &SETTINGS.devices.unshare_root,
        &SETTINGS.devices.unshare_fstype,
        mount::MountFlags::PRIVATE,
        None,
    )
    .await
    .with_context(|| {
        format!(
            "Failed to set mount propagation type to private on {}",
            SETTINGS.devices.unshare_root.display()
        )
    })?;
    Ok(())
}
