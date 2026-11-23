use screenrecord_core::CaptureRegion;

pub(super) fn dimensions(
    source: (u32, u32),
    region: Option<CaptureRegion>,
) -> Result<(u32, u32), String> {
    if !(2..=16384).contains(&source.0) || !(2..=16384).contains(&source.1) {
        return Err("Selected source has invalid dimensions.".into());
    }
    match region {
        Some(region) => {
            region.validate_bounds(source.0, source.1).map_err(|_| {
                "The selected area no longer fits inside the source. Restore its size or choose a new area.".to_owned()
            })?;
            Ok((region.width, region.height))
        }
        None => Ok((source.0 / 2 * 2, source.1 / 2 * 2)),
    }
}

pub(super) fn cursor_position(
    point: (i64, i64),
    source: (u32, u32),
    region: CaptureRegion,
) -> Option<(f64, f64)> {
    dimensions(source, Some(region)).ok()?;
    let x = point.0 - i64::from(region.x);
    let y = point.1 - i64::from(region.y);
    (x >= 0 && y >= 0 && x < i64::from(region.width) && y < i64::from(region.height)).then_some((
        x as f64 / region.width as f64,
        y as f64 / region.height as f64,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_gpu_snapshot_is_cropped_before_bottom_up_readback() {
        use windows::Win32::{
            Foundation::HMODULE,
            Graphics::{
                Direct3D::D3D_DRIVER_TYPE_WARP,
                Direct3D11::{
                    D3D11CreateDevice, D3D11_BIND_RENDER_TARGET, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
                },
                Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
            },
        };
        let _runtime = crate::cleanup::NativeRuntime::initialize().unwrap();
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .unwrap()
        };
        let (device, context) = (device.unwrap(), context.unwrap());
        let desc = D3D11_TEXTURE2D_DESC {
            Width: 8,
            Height: 6,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            ..Default::default()
        };
        let mut texture = None;
        unsafe {
            device
                .CreateTexture2D(&desc, None, Some(&mut texture))
                .unwrap()
        };
        let texture = texture.unwrap();
        let mut pixels = [255, 0, 255, 255].repeat(8 * 6);
        for y in 1..5 {
            for x in 3..7 {
                pixels[(y * 8 + x) * 4..(y * 8 + x + 1) * 4]
                    .copy_from_slice(&[x as u8, y as u8, 180, 255]);
            }
        }
        unsafe { context.UpdateSubresource(&texture, 0, None, pixels.as_ptr().cast(), 8 * 4, 0) };
        let selected = CaptureRegion {
            x: 3,
            y: 1,
            width: 4,
            height: 4,
        };
        let mut snapshot = crate::GpuSnapshot::from_source(&device, &context, desc, 4, 4).unwrap();
        snapshot
            .update_texture(&texture, (8, 6), Some(selected))
            .unwrap();
        let preserved = snapshot.bottom_up_buffer().unwrap().to_vec();
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(
                    &preserved[(y * 4 + x) * 4..(y * 4 + x + 1) * 4],
                    &[x as u8 + 3, 4 - y as u8, 180, 255]
                );
            }
        }
        pixels.fill(0);
        unsafe { context.UpdateSubresource(&texture, 0, None, pixels.as_ptr().cast(), 8 * 4, 0) };
        assert_eq!(snapshot.bottom_up_buffer().unwrap(), preserved);
        assert!(snapshot
            .update_texture(&texture, (6, 6), Some(selected))
            .is_err());
        assert_eq!(snapshot.bottom_up_buffer().unwrap(), preserved);
        snapshot
            .update_texture(&texture, (8, 6), Some(selected))
            .unwrap();
        assert!(snapshot
            .bottom_up_buffer()
            .unwrap()
            .iter()
            .all(|byte| *byte == 0));
    }

    #[test]
    fn selected_dimensions_stay_fixed_and_reject_shrinking_sources() {
        let region = CaptureRegion {
            x: 101,
            y: 57,
            width: 400,
            height: 300,
        };
        assert_eq!(dimensions((1921, 1081), None).unwrap(), (1920, 1080));
        for source in [(1920, 1080), (800, 600), (501, 357)] {
            assert_eq!(dimensions(source, Some(region)).unwrap(), (400, 300));
        }
        for source in [(500, 357), (501, 356), (0, 0), (16385, 1080)] {
            assert!(dimensions(source, Some(region)).is_err());
        }
        assert!(dimensions(
            (1920, 1080),
            Some(CaptureRegion {
                x: u32::MAX,
                ..region
            })
        )
        .is_err());
    }

    #[test]
    fn cursor_area_is_half_open_and_source_relative() {
        let region = CaptureRegion {
            x: 101,
            y: 57,
            width: 400,
            height: 300,
        };
        assert_eq!(
            cursor_position((101, 57), (1920, 1080), region),
            Some((0.0, 0.0))
        );
        assert_eq!(
            cursor_position((301, 207), (1920, 1080), region),
            Some((0.5, 0.5))
        );
        for point in [
            (100, 57),
            (101, 56),
            (501, 57),
            (101, 357),
            (i64::from(i32::MIN), 0),
        ] {
            assert_eq!(cursor_position(point, (1920, 1080), region), None);
        }
        assert_eq!(cursor_position((301, 207), (400, 300), region), None);
    }
}
