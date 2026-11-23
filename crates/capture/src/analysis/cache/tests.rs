use super::*;
use std::{io::Cursor, time::Duration};

fn write_pcm(path: &Path, rate: u32, channels: u16, frames: u64, sample: impl Fn(u64, u16) -> f32) {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for frame in 0..frames {
        for channel in 0..channels {
            writer.write_sample(sample(frame, channel)).unwrap();
        }
    }
    writer.finalize().unwrap();
}
fn input(path: &Path, duration: f64, ranges: Vec<(f64, f64)>) -> AudioAnalysisInput {
    AudioAnalysisInput {
        audio_path: path.into(),
        ffmpeg_path: std::env::current_exe().unwrap(),
        source_duration: duration,
        retained_ranges: ranges,
        bins: 127,
    }
}
fn cached(input: AudioAnalysisInput, policy: AnalysisCachePolicy) -> AudioAnalysis {
    let directory = input.audio_path.parent().unwrap().join(".audio-analysis");
    analyze_cached(
        input,
        &directory,
        policy,
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap()
}
fn reference(input: &AudioAnalysisInput) -> AudioAnalysis {
    super::super::scan(
        Cursor::new(fs::read(&input.audio_path).unwrap()),
        input,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap()
}
fn assert_same(actual: &AudioAnalysis, expected: &AudioAnalysis) {
    assert_eq!(actual.source_duration, expected.source_duration);
    assert_eq!(actual.peak, expected.peak);
    assert_eq!(actual.analyzed_seconds, expected.analyzed_seconds);
    assert!(
        (actual.rms - expected.rms).abs() < 1e-12,
        "{} != {}",
        actual.rms,
        expected.rms
    );
    assert!((actual.recommended_gain - expected.recommended_gain).abs() < 1e-12);
    assert_eq!(actual.bins.len(), expected.bins.len());
    for (a, b) in actual.bins.iter().zip(&expected.bins) {
        assert_eq!((a.min, a.max), (b.min, b.max));
    }
}
fn only_cache(directory: &Path) -> PathBuf {
    let paths: Vec<_> = fs::read_dir(directory.join(".audio-analysis"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect();
    assert_eq!(paths.len(), 1);
    paths[0].clone()
}
fn reset_counts() {
    READ_COUNTS.with(|c| c.set(ReadCounts::default()));
}
fn counts() -> ReadCounts {
    READ_COUNTS.with(|c| c.get())
}

#[test]
fn cached_waveform_and_exact_fractional_stats_match_full_scan_after_reopen() {
    for rate in [44_100, 48_000] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("system.wav");
        write_pcm(&path, rate, 2, u64::from(rate) * 3, |n, c| {
            if n % 20011 == 0 {
                0.95
            } else {
                ((n as f64 * 0.031 + f64::from(c)).sin() * 0.08) as f32
            }
        });
        let original = Sha256::digest(fs::read(&path).unwrap());
        let request = input(
            &path,
            3.2,
            vec![(0.201234, 0.452345), (1.201234, 1.452345), (2.99999, 3.2)],
        );
        let expected = reference(&request);
        assert_same(
            &cached(request.clone(), AnalysisCachePolicy::Reuse),
            &expected,
        );
        let index = only_cache(directory.path());
        let index_bytes = fs::read(&index).unwrap();
        reset_counts();
        assert_same(&cached(request, AnalysisCachePolicy::Reuse), &expected);
        assert!(counts().cache_bytes > 0);
        assert!(counts().source_bytes < 500_000);
        assert_eq!(fs::read(index).unwrap(), index_bytes);
        let changed_trim = input(&path, 3.2, vec![(0.000001, 0.125), (1.1, 1.999999)]);
        assert_same(
            &cached(changed_trim.clone(), AnalysisCachePolicy::Reuse),
            &reference(&changed_trim),
        );
        assert_eq!(Sha256::digest(fs::read(path).unwrap()), original);
    }
}

#[test]
fn silence_zero_first_channel_overlap_and_eof_have_exact_counts() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("system.wav");
    write_pcm(&path, 48_000, 2, 48_000, |n, c| {
        if c == 0 {
            0.0
        } else if n == 4096 {
            -0.9
        } else {
            -0.2
        }
    });
    for ranges in [
        vec![],
        vec![(0.0, 0.1), (0.1 - 1e-10, 0.3)],
        vec![(0.999999, 1.5)],
        vec![(1.1, 1.5)],
    ] {
        let request = input(&path, 1.5, ranges);
        assert_same(
            &cached(request.clone(), AnalysisCachePolicy::Reuse),
            &reference(&request),
        );
    }
    write_pcm(&path, 48_000, 2, 48_000, |_, _| 0.0);
    let result = cached(input(&path, 1.0, vec![]), AnalysisCachePolicy::Reuse);
    assert_eq!(
        (result.peak, result.rms, result.recommended_gain),
        (0.0, 0.0, 1.0)
    );
}

#[test]
fn corrupt_cache_rebuilds_and_refresh_rescans_original_pcm() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("system.wav");
    write_pcm(&path, 48_000, 2, 96_000, |n, _| (n % 901) as f32 / 1000.0);
    let request = input(&path, 2.0, vec![(0.11, 0.99)]);
    let expected = cached(request.clone(), AnalysisCachePolicy::Reuse);
    let cache = only_cache(directory.path());
    let valid = fs::read(&cache).unwrap();
    let mut checksum = valid.clone();
    checksum[70] ^= 0xff;
    let mut huge_header = valid.clone();
    huge_header[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut nan = valid.clone();
    let header_len = u32::from_le_bytes(valid[8..12].try_into().unwrap()) as usize;
    nan[12 + header_len..20 + header_len].copy_from_slice(&f64::NAN.to_le_bytes());
    let end = nan.len() - 32;
    let hash = Sha256::digest(&nan[..end]);
    nan[end..].copy_from_slice(&hash);
    for invalid in [
        vec![],
        b"wrong magic".to_vec(),
        valid[..valid.len() - 1].to_vec(),
        checksum,
        huge_header,
        nan,
    ] {
        fs::write(&cache, invalid).unwrap();
        assert_same(
            &cached(request.clone(), AnalysisCachePolicy::Reuse),
            &expected,
        );
        assert_eq!(fs::read(&cache).unwrap(), valid);
    }
    reset_counts();
    assert_same(&cached(request, AnalysisCachePolicy::Refresh), &expected);
    assert!(counts().source_bytes >= 96_000 * 8);
    assert_eq!(counts().cache_bytes, 0);
}

#[test]
fn replaced_and_in_place_changed_audio_invalidates_even_with_restored_mtime() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("system.wav");
    write_pcm(&path, 48_000, 2, 96_000, |_, _| 0.1);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let request = input(&path, 2.0, vec![]);
    assert_eq!(
        cached(request.clone(), AnalysisCachePolicy::Reuse).peak,
        f64::from(0.1f32)
    );
    let replacement = directory.path().join("replacement.wav");
    write_pcm(&replacement, 48_000, 2, 96_000, |_, _| 0.3);
    fs::remove_file(&path).unwrap();
    fs::rename(replacement, &path).unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert_eq!(
        cached(request.clone(), AnalysisCachePolicy::Reuse).peak,
        f64::from(0.3f32)
    );
    std::thread::sleep(Duration::from_millis(2));
    let mut file = File::options().write(true).open(&path).unwrap();
    file.seek(SeekFrom::End(-8)).unwrap();
    file.write_all(&0.9f32.to_le_bytes()).unwrap();
    file.set_modified(modified).unwrap();
    drop(file);
    assert_eq!(
        cached(request, AnalysisCachePolicy::Reuse).peak,
        f64::from(0.9f32)
    );
}

#[test]
fn stable_read_handle_denies_concurrent_data_writes_and_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("system.wav");
    write_pcm(&path, 48_000, 2, 1000, |_, _| 0.2);
    let _source = Source::open(&path).unwrap();
    assert!(OpenOptions::new().write(true).open(&path).is_err());
    assert!(fs::rename(&path, directory.path().join("renamed.wav")).is_err());
    assert!(fs::remove_file(&path).is_err());
}

#[test]
fn concurrent_metadata_change_rejects_result_without_publishing_cache() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("system.wav");
    write_pcm(&path, 48_000, 2, 48_000, |_, _| 0.2);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let changed_path = path.clone();
    let result = analyze_cached(
        input(&path, 1.0, vec![]),
        &directory.path().join(".audio-analysis"),
        AnalysisCachePolicy::Reuse,
        Arc::new(AtomicBool::new(false)),
        move |progress| {
            if progress > 0.0 && progress < 1.0 {
                OpenOptions::new()
                    .access_mode(0x100)
                    .share_mode(7)
                    .open(&changed_path)
                    .unwrap()
                    .set_modified(modified + Duration::from_secs(1))
                    .unwrap();
            }
        },
    );
    assert!(result.unwrap_err().contains("changed during analysis"));
    assert_eq!(
        fs::read_dir(directory.path().join(".audio-analysis"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn full_cache_directory_preserves_entries_and_returns_fresh_statistics() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("system.wav");
    write_pcm(&path, 48_000, 2, 1000, |_, _| 0.2);
    let cache = directory.path().join(".audio-analysis");
    fs::create_dir(&cache).unwrap();
    for entry in 0..MAX_ENTRIES {
        fs::write(
            cache.join(format!("waveform-preserve-{entry}")),
            b"preserve",
        )
        .unwrap();
    }
    let request = input(&path, 1.0, vec![]);
    assert_same(
        &cached(request.clone(), AnalysisCachePolicy::Reuse),
        &reference(&request),
    );
    assert_eq!(fs::read_dir(&cache).unwrap().count(), MAX_ENTRIES);
    for entry in fs::read_dir(cache).unwrap() {
        assert_eq!(fs::read(entry.unwrap().path()).unwrap(), b"preserve");
    }
}

#[test]
fn cancelled_scan_never_publishes_or_replaces_valid_cache() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("system.wav");
    write_pcm(&path, 48_000, 2, 48_000, |_, _| 0.2);
    let request = input(&path, 1.0, vec![]);
    let cache_directory = directory.path().join(".audio-analysis");
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    assert!(analyze_cached(
        request.clone(),
        &cache_directory,
        AnalysisCachePolicy::Reuse,
        cancel,
        move |p| {
            if p > 0.0 {
                signal.store(true, Ordering::Release);
            }
        }
    )
    .unwrap_err()
    .contains("cancelled"));
    assert_eq!(fs::read_dir(&cache_directory).unwrap().count(), 0);
    cached(request.clone(), AnalysisCachePolicy::Reuse);
    let path = only_cache(directory.path());
    let bytes = fs::read(&path).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    assert!(analyze_cached(
        request,
        &cache_directory,
        AnalysisCachePolicy::Refresh,
        cancel,
        move |p| {
            if p > 0.0 {
                signal.store(true, Ordering::Release);
            }
        }
    )
    .is_err());
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn unavailable_cache_and_directory_junction_never_redirect_writes() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let path = directory.path().join("system.wav");
    write_pcm(&path, 48_000, 2, 1000, |_, _| 0.2);
    let request = input(&path, 1.0, vec![]);
    let cache = directory.path().join(".audio-analysis");
    fs::write(&cache, b"preserve").unwrap();
    assert_same(
        &cached(request.clone(), AnalysisCachePolicy::Reuse),
        &reference(&request),
    );
    assert_eq!(fs::read(&cache).unwrap(), b"preserve");
    assert!(
        CacheDirectory::open(&directory.path().join("..").join(".audio-analysis"), &path).is_err()
    );
    fs::remove_file(&cache).unwrap();
    let status = crate::process::command(Path::new("cmd.exe"))
        .args(["/c", "mklink", "/J"])
        .arg(&cache)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert_same(
        &cached(request.clone(), AnalysisCachePolicy::Reuse),
        &reference(&request),
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    fs::remove_dir(&cache).unwrap();
}

#[test]
fn duration_and_bin_changes_rebuild_index_without_stale_waveforms() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("system.wav");
    write_pcm(
        &path,
        48_000,
        2,
        96_000,
        |n, _| if n > 48_000 { 0.8 } else { 0.2 },
    );
    cached(input(&path, 2.0, vec![]), AnalysisCachePolicy::Reuse);
    let mut request = input(&path, 0.51, vec![]);
    request.bins = 31;
    assert_same(
        &cached(request.clone(), AnalysisCachePolicy::Reuse),
        &reference(&request),
    );
    assert_same(
        &cached(request.clone(), AnalysisCachePolicy::Reuse),
        &reference(&request),
    );
}

#[test]
fn rf64_extensible_and_unsupported_pcm_formats_are_distinguished() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("system.wav");
    write_pcm(&path, 48_000, 2, 100, |_, _| 0.2);
    let original = fs::read(&path).unwrap();
    let mut file = File::open(&path).unwrap();
    let format = read_format(&mut file).unwrap().unwrap();
    let mut rf64 = Vec::new();
    rf64.extend_from_slice(b"RF64");
    rf64.extend_from_slice(&u32::MAX.to_le_bytes());
    rf64.extend_from_slice(b"WAVEds64");
    rf64.extend_from_slice(&28u32.to_le_bytes());
    rf64.extend_from_slice(&((original.len() + 36 - 8) as u64).to_le_bytes());
    rf64.extend_from_slice(&(format.frames * format.frame_bytes()).to_le_bytes());
    rf64.extend_from_slice(&format.frames.to_le_bytes());
    rf64.extend_from_slice(&0u32.to_le_bytes());
    rf64.extend_from_slice(&original[12..]);
    let data_size = format.data as usize + 36 - 4;
    rf64[data_size..data_size + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(&path, &rf64).unwrap();
    let result = cached(input(&path, 1.0, vec![]), AnalysisCachePolicy::Reuse);
    assert_eq!(result.peak, f64::from(0.2f32));
    assert_eq!(result.analyzed_seconds, 100.0 / 48_000.0);
    fs::write(&path, &rf64[..rf64.len() - 1]).unwrap();
    assert!(read_format(&mut File::open(&path).unwrap()).is_err());
    let mut pcm = original;
    let fmt = pcm.windows(4).position(|b| b == b"fmt ").unwrap() + 8;
    pcm[fmt..fmt + 2].copy_from_slice(&1u16.to_le_bytes());
    fs::write(&path, pcm).unwrap();
    assert!(read_format(&mut File::open(path).unwrap())
        .unwrap()
        .is_none());
}

#[test]
fn same_sample_edges_exclude_deleted_peak_without_rounding_leak() {
    for rate in [44_100, 48_000] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("system.wav");
        let ranges = vec![(0.201234, 0.452345), (1.201234, 1.452345)];
        let native: Vec<_> = ranges
            .iter()
            .map(|&(a, b)| (sample_at(a, rate), sample_at(b, rate)))
            .collect();
        write_pcm(&path, rate, 2, u64::from(rate) * 2, |n, _| {
            if native.iter().any(|&(a, b)| n >= a && n < b) {
                0.02
            } else {
                0.99
            }
        });
        let request = input(&path, 2.0, ranges);
        cached(input(&path, 2.0, vec![]), AnalysisCachePolicy::Reuse);
        let result = cached(request, AnalysisCachePolicy::Reuse);
        assert_eq!(result.peak, f64::from(0.02f32));
        assert!((result.rms - f64::from(0.02f32)).abs() < 1e-12);
        assert_eq!(
            result.recommended_gain,
            screenrecord_core::MAX_RECORDED_GAIN
        );
        assert_eq!(
            result.analyzed_seconds,
            native.iter().map(|&(a, b)| b - a).sum::<u64>() as f64 / f64::from(rate)
        );
        assert!(result.bins.iter().any(|bin| bin.max == f64::from(0.99f32)));
    }
}

#[test]
fn nonregular_cache_target_is_preserved_and_never_used_as_statistics() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("system.wav");
    write_pcm(&path, 48_000, 2, 1000, |_, _| 0.2);
    let request = input(&path, 1.0, vec![]);
    let expected = cached(request.clone(), AnalysisCachePolicy::Reuse);
    let cache = only_cache(directory.path());
    fs::remove_file(&cache).unwrap();
    fs::create_dir(&cache).unwrap();
    let sentinel = cache.join("preserve.txt");
    fs::write(&sentinel, b"preserve").unwrap();
    assert_same(&cached(request, AnalysisCachePolicy::Reuse), &expected);
    assert_eq!(fs::read(sentinel).unwrap(), b"preserve");
}

#[test]
#[ignore = "Uses bundled FFmpeg on synthetic PCM only."]
fn media_contract_audio_cache_preserves_pcm16_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("old.wav");
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for n in 0..44_100 {
        writer
            .write_sample(if n < 22_050 { 16384i16 } else { 1000i16 })
            .unwrap();
    }
    writer.finalize().unwrap();
    let mut request = input(&path, 1.0, vec![(0.500001, 0.999999)]);
    request.ffmpeg_path = ffmpeg();
    let expected =
        super::super::analyze(request.clone(), Arc::new(AtomicBool::new(false)), |_| {}).unwrap();
    let result = cached(request, AnalysisCachePolicy::Reuse);
    assert_same(&result, &expected);
    assert_eq!(result.peak, 1000.0 / 32768.0);
    assert!(!directory.path().join(".audio-analysis").exists());
}

fn ffmpeg() -> PathBuf {
    std::env::var_os("SCREENRECORD_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../src-tauri/resources/ffmpeg.exe")
                .canonicalize()
                .unwrap()
        })
}

#[derive(Serialize, Deserialize)]
struct Measurement {
    operation: String,
    elapsed_ms: f64,
    peak: f64,
    rms: f64,
    analyzed_seconds: f64,
    recommended_gain: f64,
    waveform_sha256: String,
    source_bytes: u64,
    cache_bytes: u64,
}
fn measured(operation: &str, request: AudioAnalysisInput) -> Measurement {
    reset_counts();
    let started = std::time::Instant::now();
    let result = if operation.starts_with("baseline") {
        super::super::analyze(request, Arc::new(AtomicBool::new(false)), |_| {}).unwrap()
    } else {
        cached(
            request,
            if operation == "build" {
                AnalysisCachePolicy::Refresh
            } else {
                AnalysisCachePolicy::Reuse
            },
        )
    };
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut hash = Sha256::new();
    for bin in &result.bins {
        hash.update(bin.min.to_le_bytes());
        hash.update(bin.max.to_le_bytes());
    }
    Measurement {
        operation: operation.into(),
        elapsed_ms,
        peak: result.peak,
        rms: result.rms,
        analyzed_seconds: result.analyzed_seconds,
        recommended_gain: result.recommended_gain,
        waveform_sha256: format!("{:x}", hash.finalize()),
        source_bytes: counts().source_bytes,
        cache_bytes: counts().cache_bytes,
    }
}
fn benchmark_input(path: &Path, trimmed: bool) -> AudioAnalysisInput {
    let ranges = if trimmed {
        vec![
            (0.201234, 17.452345),
            (100.0023, 1234.9977),
            (1800.123456, 3599.812345),
        ]
    } else {
        vec![]
    };
    let mut request = input(path, 3600.0, ranges);
    request.ffmpeg_path = ffmpeg();
    request.bins = 4096;
    request
}

#[test]
#[ignore = "Helper used only by the one-hour synthetic benchmark."]
fn benchmark_reopen_child() {
    let Some(path) = std::env::var_os("SCREENRECORD_CACHE_BENCH_SOURCE") else {
        return;
    };
    let operation = std::env::var("SCREENRECORD_CACHE_BENCH_OPERATION").unwrap();
    let result = measured(
        &operation,
        benchmark_input(Path::new(&path), operation.contains("trim")),
    );
    println!(
        "CACHE_BENCH_RESULT={}",
        serde_json::to_string(&result).unwrap()
    );
}

fn fresh_process(path: &Path, operation: &str) -> (Measurement, f64) {
    use std::process::Stdio;
    let started = std::time::Instant::now();
    let mut child = crate::process::command(&std::env::current_exe().unwrap())
        .args([
            "--exact",
            "analysis::cache::tests::benchmark_reopen_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("SCREENRECORD_CACHE_BENCH_SOURCE", path)
        .env("SCREENRECORD_CACHE_BENCH_OPERATION", operation)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _job = crate::process::ChildJob::attach(&child).unwrap();
    let status = crate::process::wait_bounded(&mut child, Duration::from_secs(180)).unwrap();
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(status.success(), "{stdout}\n{stderr}");
    let result = stdout
        .lines()
        .find_map(|line| {
            line.split_once("CACHE_BENCH_RESULT=")
                .map(|(_, value)| value)
        })
        .unwrap_or_else(|| panic!("Missing benchmark result: {stdout}\n{stderr}"));
    (
        serde_json::from_str(result).unwrap(),
        started.elapsed().as_secs_f64() * 1000.0,
    )
}

#[test]
#[ignore = "Generates 1.38 GB synthetic PCM on C: temp; release benchmark, no devices."]
fn benchmark_one_hour_cache() {
    if cfg!(debug_assertions) {
        panic!("Run this benchmark with --release.");
    }
    let temp_root = std::env::temp_dir();
    assert!(temp_root
        .to_string_lossy()
        .to_ascii_lowercase()
        .starts_with("c:"));
    let directory = tempfile::Builder::new()
        .prefix("screenrecord-audio-cache-bench-")
        .tempdir_in(temp_root)
        .unwrap();
    let path = directory.path().join("system.wav");
    let mut writer = BufWriter::with_capacity(1024 * 1024, File::create(&path).unwrap());
    let frames = 3600u64 * 48_000;
    let size = frames * 8;
    writer.write_all(b"RIFF").unwrap();
    writer
        .write_all(&((size + 36) as u32).to_le_bytes())
        .unwrap();
    writer.write_all(b"WAVEfmt ").unwrap();
    writer.write_all(&16u32.to_le_bytes()).unwrap();
    writer.write_all(&3u16.to_le_bytes()).unwrap();
    writer.write_all(&2u16.to_le_bytes()).unwrap();
    writer.write_all(&48_000u32.to_le_bytes()).unwrap();
    writer.write_all(&(48_000u32 * 8).to_le_bytes()).unwrap();
    writer.write_all(&8u16.to_le_bytes()).unwrap();
    writer.write_all(&32u16.to_le_bytes()).unwrap();
    writer.write_all(b"data").unwrap();
    writer.write_all(&(size as u32).to_le_bytes()).unwrap();
    let mut second = vec![0u8; 48_000 * 8];
    for n in 0..48_000 {
        for c in 0..2 {
            let sample = (((n as f64 * 0.031 + c as f64).sin()) * 0.06) as f32;
            second[(n * 2 + c) * 4..(n * 2 + c + 1) * 4].copy_from_slice(&sample.to_le_bytes());
        }
    }
    for seconds in 0..3600 {
        if seconds % 137 == 0 {
            second[..4].copy_from_slice(&0.95f32.to_le_bytes());
        } else {
            second[..4].copy_from_slice(&0f32.to_le_bytes());
        }
        writer.write_all(&second).unwrap();
    }
    writer.flush().unwrap();
    writer.get_ref().sync_all().unwrap();
    drop(writer);
    let source_stamp = Source::open(&path).unwrap().stamp().unwrap();
    let build = measured("build", benchmark_input(&path, false));
    let cache_size = fs::metadata(only_cache(directory.path())).unwrap().len();
    let mut rounds = Vec::new();
    for round in 0..3 {
        let operations = if round % 2 == 0 {
            ["baseline-open", "reopen", "baseline-trim", "trim"]
        } else {
            ["trim", "baseline-trim", "reopen", "baseline-open"]
        };
        let mut measurements = Vec::new();
        for operation in operations {
            let (measurement, process_elapsed_ms) = fresh_process(&path, operation);
            assert_eq!(measurement.waveform_sha256, build.waveform_sha256);
            if !operation.starts_with("baseline") {
                assert!(measurement.cache_bytes > 0);
                assert!(measurement.source_bytes < 500_000);
            }
            measurements.push(serde_json::json!({"measurement":measurement,"process_elapsed_ms":process_elapsed_ms}));
        }
        for (a, b) in [("baseline-open", "reopen"), ("baseline-trim", "trim")] {
            let left = &measurements
                .iter()
                .find(|m| m["measurement"]["operation"] == a)
                .unwrap()["measurement"];
            let right = &measurements
                .iter()
                .find(|m| m["measurement"]["operation"] == b)
                .unwrap()["measurement"];
            for key in ["peak", "rms", "analyzed_seconds", "recommended_gain"] {
                assert!(
                    (left[key].as_f64().unwrap() - right[key].as_f64().unwrap()).abs() < 1e-12,
                    "{key}: {left} {right}"
                );
            }
        }
        rounds.push(serde_json::json!({"round":round+1,"results":measurements}));
    }
    assert_eq!(Source::open(&path).unwrap().stamp().unwrap(), source_stamp);
    let report = serde_json::json!({"profile":"release","sample_rate":48000,"channels":2,"duration_seconds":3600,"source_bytes":fs::metadata(&path).unwrap().len(),"source_drive":"C:","cache_bytes":cache_size,"build":build,"rounds":rounds,"notes":"Fresh process per query; warm OS filesystem cache; source/cache byte counts are logical instrumentation, exclude RIFF header reads; baseline decodes the complete source through FFmpeg; no CPU/RAM or cold-storage claim; fixture generation excluded."});
    println!(
        "ONE_HOUR_CACHE_BENCH={}",
        serde_json::to_string_pretty(&report).unwrap()
    );
    if let Some(report_path) = std::env::var_os("SCREENRECORD_CACHE_BENCH_REPORT") {
        fs::write(report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    directory.close().unwrap();
}
