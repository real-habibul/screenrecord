use screenrecord_core::CaptureDevices;

#[derive(Default)]
pub struct DeviceCatalog {
    generation: u64,
    cameras: Vec<String>,
    microphones: Vec<String>,
}

impl DeviceCatalog {
    pub fn reserve(&mut self) -> Result<u64, String> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Device refresh limit reached. Restart the application.")?;
        self.cameras.clear();
        self.microphones.clear();
        Ok(self.generation)
    }

    pub fn publish(&mut self, generation: u64, devices: &CaptureDevices) -> Result<(), String> {
        if generation != self.generation {
            return Err("Device refresh was superseded by a newer request.".into());
        }
        self.cameras = devices
            .cameras
            .iter()
            .map(|device| device.id.clone())
            .collect();
        self.microphones = devices
            .microphones
            .iter()
            .map(|device| device.id.clone())
            .collect();
        Ok(())
    }

    pub fn camera_available(&self, id: &str) -> bool {
        self.cameras.iter().any(|known| known == id)
    }

    pub fn microphone_available(&self, id: &str) -> bool {
        self.microphones.iter().any(|known| known == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use screenrecord_core::Device;

    fn devices(id: &str) -> CaptureDevices {
        CaptureDevices {
            sources: vec![],
            apps: vec![],
            cameras: vec![Device {
                id: format!("camera-{id}"),
                name: id.into(),
            }],
            microphones: vec![Device {
                id: format!("microphone-{id}"),
                name: id.into(),
            }],
        }
    }

    #[test]
    fn delayed_old_enumeration_cannot_replace_new_camera_or_microphone() {
        let mut catalog = DeviceCatalog::default();
        let old = catalog.reserve().unwrap();
        let new = catalog.reserve().unwrap();
        catalog.publish(new, &devices("new")).unwrap();
        assert!(catalog.publish(old, &devices("old")).is_err());
        assert!(catalog.camera_available("camera-new"));
        assert!(catalog.microphone_available("microphone-new"));
        assert!(!catalog.camera_available("camera-old"));
        assert!(!catalog.microphone_available("microphone-old"));
    }

    #[test]
    fn failed_refresh_does_not_leave_old_device_authorizations() {
        let mut catalog = DeviceCatalog::default();
        let first = catalog.reserve().unwrap();
        catalog.publish(first, &devices("old")).unwrap();
        catalog.reserve().unwrap();
        assert!(!catalog.camera_available("camera-old"));
        assert!(!catalog.microphone_available("microphone-old"));
        assert!(catalog.publish(first, &devices("old")).is_err());
    }
}
