pub fn assert_volume_full() {
    let distro = std::env::var("SCREENRECORD_TEST_QUOTA_DISTRO").unwrap();
    let mount = std::env::var("SCREENRECORD_TEST_QUOTA_MOUNT").unwrap();
    let result = std::process::Command::new("wsl.exe")
        .args(["-d", &distro, "--exec", "stat", "-f", "-c", "%T %a", &mount])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(
        std::str::from_utf8(&result.stdout).unwrap().trim(),
        "tmpfs 0"
    );
}
