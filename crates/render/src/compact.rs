use screenrecord_core::WHATSAPP_MIN_VIDEO_BITRATE;

pub(crate) const SIZE_ERROR: &str =
    "The video did not fit within 16 MB. Shorten the video or choose Standard.";

pub(crate) fn retry_bitrate(
    current: u32,
    actual_bytes: u64,
    maximum_bytes: u64,
    duration: f64,
    has_audio: bool,
) -> Result<u32, String> {
    let fixed = 65_536.0
        + if has_audio {
            duration * 96_000.0 / 8.0
        } else {
            0.0
        };
    let available = maximum_bytes as f64 * 0.9 - fixed;
    let measured = actual_bytes as f64 - fixed;
    if available <= 0.0 || measured <= 0.0 {
        return Err(SIZE_ERROR.into());
    }
    let ratio = (available / measured).min(0.8);
    let next = ((current as f64 * ratio / 1000.0).floor() * 1000.0)
        .max(f64::from(WHATSAPP_MIN_VIDEO_BITRATE));
    if !next.is_finite()
        || next < f64::from(WHATSAPP_MIN_VIDEO_BITRATE)
        || next >= f64::from(current)
    {
        return Err(SIZE_ERROR.into());
    }
    Ok(next as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn size_retry_lowers_rate_with_audio_reserve_and_never_crosses_the_floor() {
        let next = retry_bitrate(1_000_000, 18_000_000, 16_000_000, 100.0, true).unwrap();
        assert!((WHATSAPP_MIN_VIDEO_BITRATE..=800_000).contains(&next));
        assert!(next < retry_bitrate(1_000_000, 18_000_000, 16_000_000, 100.0, false).unwrap());
        assert!(retry_bitrate(
            WHATSAPP_MIN_VIDEO_BITRATE,
            16_000_001,
            16_000_000,
            100.0,
            true
        )
        .is_err());
        assert!(retry_bitrate(2_400_000, 1_000_000, 10_000, 3.0, true).is_err());
        assert_eq!(
            retry_bitrate(321_000, 18_000_000, 16_000_000, 100.0, true).unwrap(),
            WHATSAPP_MIN_VIDEO_BITRATE
        );
    }
}
