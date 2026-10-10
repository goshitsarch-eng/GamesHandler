use gamehandler_desktop::request_json;
use serde_json::{Value, json};
use tempfile::TempDir;

fn call(root: &TempDir, op: &str, args: Option<Value>) -> Value {
    let mut request = json!({"context":{"config":root.path().join("config"),"data":root.path().join("data")},"operation":op});
    if let Some(args) = args {
        request["arguments"] = args;
    }
    request_json(request.to_string().as_bytes())
}

#[test]
fn unicode_records_round_trip_and_removal_keeps_executables() {
    let root = TempDir::new().unwrap();
    let exe = root.path().join("遊戯 space.exe");
    std::fs::write(&exe, b"fixture").unwrap();
    let template = call(&root, "new_game", None);
    assert_eq!(template["ok"], true);
    let mut game = template["data"].clone();
    game["name"] = json!("Été 遊戯 🎮");
    game["exe_path"] = json!(exe);
    let saved = call(&root, "save_game", Some(json!({"game":game})));
    assert_eq!(saved["ok"], true, "{saved}");
    let snapshot = call(
        &root,
        "snapshot",
        Some(json!({"query":"été","category":"All","sort":"name"})),
    );
    assert_eq!(snapshot["data"]["games"][0]["name"], game["name"]);
    assert_eq!(snapshot["data"]["games"][0]["id"], game["id"]);
    assert_eq!(
        call(&root, "remove_game", Some(json!({"id":game["id"]})))["ok"],
        true
    );
    assert!(exe.exists());
}

#[test]
fn corrupted_library_and_settings_are_never_overwritten() {
    let root = TempDir::new().unwrap();
    let config = root.path().join("config");
    std::fs::create_dir(&config).unwrap();
    std::fs::write(config.join("games.json"), b"broken library").unwrap();
    let failed = call(&root, "remove_game", Some(json!({"id":"missing"})));
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["kind"], "corrupt_data");
    assert_eq!(
        std::fs::read(config.join("games.json")).unwrap(),
        b"broken library"
    );
    std::fs::write(config.join("settings.json"), b"{broken}").unwrap();
    assert_eq!(
        call(
            &root,
            "save_settings",
            Some(json!({"settings":{"color_scheme":"light"}}))
        )["ok"],
        false
    );
    assert_eq!(
        std::fs::read(config.join("settings.json")).unwrap(),
        b"{broken}"
    );
}

#[test]
fn bad_requests_and_unknown_operations_are_typed_errors() {
    assert_eq!(request_json(b"not json")["error"]["kind"], "validation");
    let root = TempDir::new().unwrap();
    assert_eq!(
        call(&root, "invented_operation", None)["error"]["kind"],
        "validation"
    );
    assert_eq!(
        call(&root, "launch", Some(json!({"id":"missing"})))["ok"],
        false
    );
}

#[test]
fn ffi_buffers_are_owned_and_errors_survive_the_boundary() {
    use std::ffi::CStr;
    let source = b"invalid";
    // SAFETY: source stays live; each owned response is read and freed once.
    unsafe {
        for _ in 0..100 {
            let output = gamehandler_desktop::gamehandler_request(source.as_ptr(), source.len());
            let response: Value =
                serde_json::from_slice(CStr::from_ptr(output).to_bytes()).unwrap();
            assert_eq!(response["ok"], false);
            gamehandler_desktop::gamehandler_free(output);
        }
        let output = gamehandler_desktop::gamehandler_request(std::ptr::null(), 0);
        assert!(!output.is_null());
        gamehandler_desktop::gamehandler_free(output);
    }
}

#[test]
fn native_launch_updates_play_time_and_invalid_executable_is_rejected() {
    let root = TempDir::new().unwrap();
    let mut game = call(&root, "new_game", None)["data"].clone();
    game["name"] = json!("Native fixture");
    let executable = if cfg!(target_os = "windows") {
        std::path::PathBuf::from(std::env::var("SystemRoot").expect("Windows system root"))
            .join("System32/cmd.exe")
    } else if cfg!(target_os = "macos") {
        std::path::PathBuf::from("/usr/bin/true")
    } else {
        std::path::PathBuf::from("/bin/true")
    };
    game["exe_path"] = json!(executable);
    game["kind"] = json!(if cfg!(target_os = "windows") {
        "windows"
    } else {
        "native"
    });
    if cfg!(target_os = "windows") {
        game["arguments"] = json!("/C exit 0");
    }
    assert_eq!(
        call(&root, "save_game", Some(json!({"game":game})))["ok"],
        true
    );
    assert_eq!(
        call(&root, "launch", Some(json!({"id":game["id"]})))["ok"],
        true
    );
    let snapshot = call(&root, "snapshot", Some(json!({})));
    assert!(
        snapshot["data"]["games"][0]["last_played"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    game["exe_path"] = json!(root.path().join("missing"));
    assert_eq!(
        call(&root, "save_game", Some(json!({"game":game})))["ok"],
        false
    );
}

#[cfg(target_os = "linux")]
#[test]
fn runner_assets_match_architecture_even_when_arm_asset_is_first() {
    use gamehandler_core::runners::families::{family_by_id, pick_asset_for_arch};
    let family = family_by_id("proton-ge").unwrap();
    let assets = vec![
        json!({"name":"GE-Proton-aarch64.tar.gz"}),
        json!({"name":"GE-Proton.tar.gz"}),
    ];
    assert_eq!(
        pick_asset_for_arch(&assets, family, "x86_64").unwrap()["name"],
        "GE-Proton.tar.gz"
    );
    assert_eq!(
        pick_asset_for_arch(&assets, family, "aarch64").unwrap()["name"],
        "GE-Proton-aarch64.tar.gz"
    );
}

#[test]
fn custom_cover_is_imported_into_managed_storage_without_removing_source() {
    let root = TempDir::new().unwrap();
    let executable = root.path().join("game.exe");
    std::fs::write(&executable, b"fixture").unwrap();
    let source = root.path().join("Custom artwork 遊戯.png");
    let image = include_bytes!("../../data/icons/hicolor/64x64/apps/com.goshapps.GameHandler.png");
    std::fs::write(&source, image).unwrap();
    let mut game = call(&root, "new_game", None)["data"].clone();
    game["name"] = json!("Artwork fixture");
    game["exe_path"] = json!(executable);
    game["cover_path"] = json!(source);
    let saved = call(&root, "save_game", Some(json!({"game":game})));
    assert_eq!(saved["ok"], true, "{saved}");
    let managed = std::path::PathBuf::from(saved["data"]["cover_path"].as_str().unwrap());
    assert!(managed.starts_with(root.path().join("data/covers")));
    assert_eq!(std::fs::read(managed).unwrap(), image);
    assert_eq!(std::fs::read(&source).unwrap(), image);
    let managed = std::path::PathBuf::from(saved["data"]["cover_path"].as_str().unwrap());
    for invalid in [b"not an image".as_slice(), &image[..24]] {
        std::fs::write(&source, invalid).unwrap();
        let rejected = call(
            &root,
            "import_cover",
            Some(json!({"id":saved["data"]["id"],"path":source})),
        );
        assert_eq!(rejected["ok"], false, "{rejected}");
        let mut edited = saved["data"].clone();
        edited["cover_path"] = json!(source);
        let rejected = call(&root, "save_game", Some(json!({"game":edited})));
        assert_eq!(rejected["ok"], false, "{rejected}");
        let snapshot = call(
            &root,
            "snapshot",
            Some(json!({"query":"", "category":"All", "sort":"name"})),
        );
        assert_eq!(snapshot["ok"], true, "{snapshot}");
        assert_eq!(
            snapshot["data"]["games"][0]["cover_path"],
            saved["data"]["cover_path"]
        );
        assert_eq!(std::fs::read(&managed).unwrap(), image);
        assert_eq!(std::fs::read(&source).unwrap(), invalid);
    }
    let oversized = std::fs::File::create(&source).unwrap();
    oversized.set_len(32 * 1024 * 1024 + 1).unwrap();
    let rejected = call(
        &root,
        "import_cover",
        Some(json!({"id":saved["data"]["id"],"path":source})),
    );
    assert_eq!(rejected["ok"], false, "{rejected}");
    assert_eq!(std::fs::read(&managed).unwrap(), image);
    assert!(
        std::fs::read_dir(root.path().join("data/covers"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".cover-import-"))
    );
}

#[cfg(target_os = "linux")]
#[test]
fn delayed_native_failure_is_reported_by_launch_observer() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let executable = root.path().join("delayed-failure");
    std::fs::write(
        &executable,
        "#!/bin/sh\nsleep 0.4\necho 'delayed fixture failure' >&2\nexit 7\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut game = call(&root, "new_game", None)["data"].clone();
    game["name"] = json!("Delayed failure");
    game["kind"] = json!("linux");
    game["exe_path"] = json!(executable);
    assert_eq!(
        call(&root, "save_game", Some(json!({"game":game})))["ok"],
        true
    );
    let launch = call(&root, "launch", Some(json!({"id":game["id"]})));
    assert_eq!(launch["ok"], true, "{launch}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let snapshot = call(&root, "jobs", None);
        let failed = snapshot["data"].as_array().unwrap().iter().find(|j| {
            j["kind"] == "launch" && j["payload"]["id"] == game["id"] && j["status"] == "failed"
        });
        if let Some(job) = failed {
            assert!(
                job["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("delayed fixture failure")
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Launch failure was not reported: {snapshot}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn legacy_id_cannot_escape_the_default_prefix_directory() {
    let root = TempDir::new().unwrap();
    let executable = root.path().join("fixture.exe");
    std::fs::write(&executable, b"fixture").unwrap();
    let mut game = call(&root, "new_game", None)["data"].clone();
    game["id"] = json!("../../outside");
    game["name"] = json!("Legacy malformed id");
    game["exe_path"] = json!(executable);
    assert_eq!(
        call(&root, "save_game", Some(json!({"game":game})))["ok"],
        true
    );
    let result = call(&root, "prefix_path", Some(json!({"id":game["id"]})));
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"]["kind"], "validation");
    assert!(!root.path().join("outside").exists());
    assert!(root.path().join("config/games.json").is_file());
}

#[test]
fn cli_directory_options_reopen_the_selected_library_without_inherited_overrides() {
    let root = tempfile::Builder::new()
        .prefix("GameHandler café space ")
        .tempdir()
        .unwrap();
    let exe = root.path().join("fixture");
    std::fs::write(&exe, b"fixture").unwrap();
    let mut game = call(&root, "new_game", None)["data"].clone();
    game["name"] = json!("Shortcut context 遊戯");
    game["exe_path"] = json!(exe);
    assert_eq!(
        call(&root, "save_game", Some(json!({"game":game})))["ok"],
        true
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_gamehandler-cli"))
        .env_remove("GAMEHANDLER_CONFIG_HOME")
        .env_remove("GAMEHANDLER_DATA_HOME")
        .arg("--config-dir")
        .arg(root.path().join("config"))
        .arg("--data-dir")
        .arg(root.path().join("data"))
        .arg("--list")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Shortcut context 遊戯"));
    let invalid = std::process::Command::new(env!("CARGO_BIN_EXE_gamehandler-cli"))
        .args(["--config-dir", "relative-path", "--list"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("absolute path"));
}

#[test]
fn ordinary_directories_cannot_be_saved_as_executables() {
    let root = TempDir::new().unwrap();
    let folder = root.path().join("Not an executable");
    std::fs::create_dir(&folder).unwrap();
    let mut game = call(&root, "new_game", None)["data"].clone();
    game["name"] = json!("Directory input");
    game["exe_path"] = json!(folder);
    for kind in ["windows", "linux", "native", "macos"] {
        game["kind"] = json!(kind);
        let response = call(&root, "save_game", Some(json!({"game":game})));
        assert_eq!(response["ok"], false, "{response}");
    }
    assert!(!root.path().join("config/games.json").exists());
    let bundle = root.path().join("Example.app");
    std::fs::create_dir(&bundle).unwrap();
    game["exe_path"] = json!(bundle);
    game["kind"] = json!("macos");
    assert_eq!(
        call(&root, "save_game", Some(json!({"game":game})))["ok"],
        true
    );
}
