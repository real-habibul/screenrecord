use super::*;
use screenrecord_capture::tracking::{TrackingInput, TrackingRect};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MaskTrackingProgress {
    job_id: String,
    project_id: String,
    region_id: String,
    state: String,
    progress: f64,
    error: Option<String>,
    tracking: Option<MaskTracking>,
}

fn publish(app: &tauri::AppHandle, value: MaskTrackingProgress) {
    let state = app.state::<AppState>();
    if let Ok(mut progress) = state.mask_tracking_progress.lock() {
        if !progress
            .as_ref()
            .is_some_and(|current| current.job_id == value.job_id)
        {
            return;
        }
        *progress = Some(value.clone());
    }
    let _ = app.emit("mask-tracking-progress", value);
}

fn tracking_request(
    project: &Project,
    region_id: &str,
    start: Option<f64>,
    engine: &Path,
) -> Result<(TrackingInput, Redaction), String> {
    let region = project
        .edits
        .redactions
        .iter()
        .find(|region| region.id == region_id)
        .ok_or("Select an existing mask region.")?
        .clone();
    let start = start.unwrap_or(region.start);
    if !start.is_finite()
        || start < region.start
        || start + 0.1 > region.end + TIME_EPSILON
        || region.end - region.start > MAX_MASK_TRACKING_SECONDS + TIME_EPSILON
    {
        return Err("Track a mask range of at most 300 seconds, starting at least 0.1 seconds before its end.".into());
    }
    if start > region.start + TIME_EPSILON
        && !region.tracking.as_ref().is_some_and(|tracking| {
            tracking
                .keyframes
                .iter()
                .any(|key| key.kind == "manual" && (key.time - start).abs() <= TIME_EPSILON)
        })
    {
        return Err("Add a manual keyframe at this position before retracking.".into());
    }
    let rect = if let Some(key) = region.tracking.as_ref().and_then(|tracking| {
        tracking
            .keyframes
            .iter()
            .find(|key| key.kind == "manual" && (key.time - start).abs() <= TIME_EPSILON)
    }) {
        TrackingRect {
            x: key.x,
            y: key.y,
            width: key.width,
            height: key.height,
        }
    } else {
        TrackingRect {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        }
    };
    Ok((
        TrackingInput {
            source_path: PathBuf::from(&project.screen_path),
            ffmpeg_path: engine.to_owned(),
            source_width: project.width,
            source_height: project.height,
            start,
            end: region.end,
            rect,
        },
        region,
    ))
}

fn merge_tracking(
    project: &Project,
    mut region: Redaction,
    start: f64,
    keys: Vec<MaskKeyframe>,
) -> Result<MaskTracking, String> {
    if keys.first().is_none_or(|key| {
        (key.time - start).abs() > TIME_EPSILON || !["manual", "lost"].contains(&key.kind.as_str())
    }) {
        return Err("The tracker did not return a valid keyframe at its requested start.".into());
    }
    let mut prefix = region
        .tracking
        .as_ref()
        .map(|tracking| {
            tracking
                .keyframes
                .iter()
                .filter(|key| key.time < start)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    prefix.extend(keys);
    let tracking = MaskTracking {
        keyframes: prefix,
        reviewed: false,
    };
    region.tracking = Some(tracking.clone());
    let mut edits = project.edits.clone();
    let target = edits
        .redactions
        .iter_mut()
        .find(|target| target.id == region.id)
        .ok_or("The mask no longer exists.")?;
    *target = region;
    edits.validate(project.duration)?;
    Ok(tracking)
}

#[tauri::command]
pub(super) async fn start_mask_tracking(
    app: tauri::AppHandle,
    id: String,
    region_id: String,
    job_id: String,
    start: Option<f64>,
) -> Result<(), String> {
    validate_id(&id)?;
    validate_id(&job_id)?;
    if region_id.is_empty() || region_id.len() > 128 || region_id.chars().any(char::is_control) {
        return Err("Select a valid mask region.".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let state = app.state::<AppState>();
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "A media operation is starting. Try again shortly.")?;
        if busy(&state) {
            return Err("Wait for the current media operation before tracking a mask.".into());
        }
        *state
            .mask_tracking_cancel
            .lock()
            .map_err(|e| e.to_string())? = Some((job_id.clone(), cancel.clone()));
        *state
            .mask_tracking_progress
            .lock()
            .map_err(|e| e.to_string())? = Some(MaskTrackingProgress {
            job_id: job_id.clone(),
            project_id: id.clone(),
            region_id: region_id.clone(),
            state: "running".into(),
            progress: 0.0,
            error: None,
            tracking: None,
        });
        state.editor_busy.store(true, Ordering::Release);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let busy = BusyReset(&state.editor_busy);
        let reporter = app.clone();
        let reporter_job = job_id.clone();
        let reporter_project = id.clone();
        let reporter_region = region_id.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if cancel.load(Ordering::Acquire) {
                return Err("Mask tracking cancelled.".into());
            }
            let project = load_project_metadata(&state.root, &id)?;
            let (input, region) = tracking_request(&project, &region_id, start, ffmpeg(&state)?)?;
            let start = input.start;
            let result =
                screenrecord_capture::tracking::track(input, cancel.clone(), move |progress| {
                    publish(
                        &reporter,
                        MaskTrackingProgress {
                            job_id: reporter_job.clone(),
                            project_id: reporter_project.clone(),
                            region_id: reporter_region.clone(),
                            state: "running".into(),
                            progress,
                            error: None,
                            tracking: None,
                        },
                    );
                })?;
            merge_tracking(&project, region, start, result.keyframes)
        }))
        .unwrap_or_else(|_| {
            Err("Mask tracking worker failed. Original media is preserved.".into())
        });
        let cancelled = cancel.load(Ordering::Acquire);
        let (name, error, tracking) = if cancelled {
            ("cancelled", None, None)
        } else {
            match result {
                Ok(tracking) => ("completed", None, Some(tracking)),
                Err(error) => ("failed", Some(error), None),
            }
        };
        if let Ok(mut slot) = state.mask_tracking_cancel.lock() {
            if slot.as_ref().is_some_and(|(current, _)| current == &job_id) {
                *slot = None;
            }
        }
        drop(busy);
        publish(
            &app,
            MaskTrackingProgress {
                job_id,
                project_id: id,
                region_id,
                state: name.into(),
                progress: if name == "completed" { 1.0 } else { 0.0 },
                error,
                tracking,
            },
        );
    });
    Ok(())
}

#[tauri::command]
pub(super) fn cancel_mask_tracking(
    state: tauri::State<'_, AppState>,
    job_id: String,
) -> Result<(), String> {
    validate_id(&job_id)?;
    if let Some((current, cancel)) = state
        .mask_tracking_cancel
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
    {
        if current == &job_id {
            cancel.store(true, Ordering::Release);
        }
    }
    Ok(())
}

#[tauri::command]
pub(super) fn get_mask_tracking_progress(
    state: tauri::State<'_, AppState>,
) -> Option<MaskTrackingProgress> {
    state
        .mask_tracking_progress
        .lock()
        .ok()
        .and_then(|progress| progress.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(time: f64, x: f64, kind: &str) -> MaskKeyframe {
        MaskKeyframe {
            time,
            x,
            y: 0.1,
            width: 0.2,
            height: 0.1,
            confidence: 1.0,
            kind: kind.into(),
        }
    }
    fn project() -> Project {
        let mut edits = EditSettings::new(10.0);
        edits.redactions.push(Redaction {
            id: "mask".into(),
            start: 1.0,
            end: 5.0,
            x: 0.1,
            y: 0.1,
            width: 0.2,
            height: 0.1,
            mode: "solid".into(),
            strength: 16.0,
            tracking: None,
        });
        Project {
            version: PROJECT_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name: "Tracking fixture".into(),
            created_at: "2026-11-12T00:00:00Z".into(),
            directory: "owned-project".into(),
            screen_path: "owned-project/screen.mp4".into(),
            microphone_path: None,
            system_audio_path: None,
            camera_path: None,
            camera_offset_seconds: 0.0,
            camera_intervals: None,
            width: 1920,
            height: 1080,
            fps: 30,
            duration: 10.0,
            cursor: vec![],
            edits,
            status: "ready".into(),
        }
    }
    #[test]
    fn tracking_input_resolves_owned_screen_and_requires_manual_retrack_anchor() {
        let mut project = project();
        let engine = Path::new("owned-ffmpeg.exe");
        let (input, _) = tracking_request(&project, "mask", None, engine).unwrap();
        assert_eq!(input.source_path, PathBuf::from(&project.screen_path));
        assert_eq!(
            (
                input.start,
                input.end,
                input.source_width,
                input.source_height
            ),
            (1.0, 5.0, 1920, 1080)
        );
        for start in [0.5, 2.0, 5.0, f64::NAN] {
            assert!(tracking_request(&project, "mask", Some(start), engine).is_err());
        }
        assert!(tracking_request(&project, "missing", None, engine).is_err());
        project.edits.redactions[0].tracking = Some(MaskTracking {
            reviewed: true,
            keyframes: vec![
                key(1.0, 0.1, "manual"),
                key(2.0, 0.6, "manual"),
                key(3.0, 0.7, "auto"),
            ],
        });
        let (input, _) = tracking_request(&project, "mask", Some(2.0), engine).unwrap();
        assert_eq!(input.rect.x, 0.6);
        assert!(tracking_request(&project, "mask", Some(3.0), engine).is_err());
    }
    #[test]
    fn retracking_replaces_tail_preserves_prefix_and_resets_review_without_mutation() {
        let mut project = project();
        project.edits.redactions[0].tracking = Some(MaskTracking {
            reviewed: true,
            keyframes: vec![
                key(1.0, 0.1, "manual"),
                key(2.0, 0.6, "manual"),
                key(3.0, 0.7, "auto"),
            ],
        });
        let region = project.edits.redactions[0].clone();
        let output = merge_tracking(
            &project,
            region.clone(),
            2.0,
            vec![key(2.0, 0.6, "manual"), key(2.1, 0.65, "auto")],
        )
        .unwrap();
        assert!(!output.reviewed);
        assert_eq!(
            output
                .keyframes
                .iter()
                .map(|key| key.time)
                .collect::<Vec<_>>(),
            vec![1.0, 2.0, 2.1]
        );
        assert!(
            project.edits.redactions[0]
                .tracking
                .as_ref()
                .unwrap()
                .reviewed
        );
        assert!(merge_tracking(&project, region.clone(), 2.0, vec![]).is_err());
        assert!(merge_tracking(
            &project,
            region,
            2.0,
            vec![key(1.5, 0.3, "auto"), key(1.4, 0.4, "auto")]
        )
        .is_err());
    }
}
