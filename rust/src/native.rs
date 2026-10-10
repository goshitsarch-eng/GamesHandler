use crate::{
    api::{Context, Failure, Result, error},
    jobs::Job,
};
use gamehandler_core::models::Game;
use serde_json::{Value, json};
use std::path::Path;
#[path = "../../crates/app/src/http.rs"]
pub(crate) mod http;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
#[path = "../../crates/core/src/runners/shell.rs"]
mod arguments;

#[cfg(any(target_os = "windows", test))]
#[path = "windows_arguments.rs"]
mod windows_arguments;

pub fn launch(ctx: &Context, g: &Game) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        crate::linux::launch(ctx, g)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = ctx;
        if cfg!(target_os = "macos") && g.kind == "windows" {
            return Err(Failure::validation(
                "This Windows game needs a macOS compatibility runner; add a native macOS application instead.",
            ));
        }
        #[cfg(target_os = "windows")]
        let args = windows_arguments::split(&g.arguments).map_err(error)?;
        #[cfg(not(target_os = "windows"))]
        let args = arguments::split_posix(&g.arguments).map_err(error)?;
        let mut command = if cfg!(target_os = "macos") && g.exe_path.ends_with(".app") {
            let mut c = std::process::Command::new("/usr/bin/open");
            c.arg("-a").arg(&g.exe_path).arg("--args");
            c
        } else {
            std::process::Command::new(&g.exe_path)
        };
        command.args(args);
        if !g.working_directory.is_empty() {
            command.current_dir(&g.working_directory);
        }
        for line in g.environment.lines() {
            if let Some((k, v)) = line.split_once('=')
                && !k.trim().is_empty()
            {
                command.env(k.trim(), v);
            }
        }
        if !g.additional_app.trim().is_empty() {
            let mut helper = std::process::Command::new(&g.additional_app).spawn()?;
            std::thread::spawn(move || {
                let _ = helper.wait();
            });
        }
        let mut child = command.spawn()?;
        std::thread::sleep(std::time::Duration::from_millis(200));
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(error(format!(
                    "The application exited immediately with {status}"
                )));
            }
            return Ok(());
        }
        crate::jobs::observe_launch(g.id.clone(), move || {
            let status = child.wait()?;
            if !status.success() {
                return Err(error(format!("The application exited with {status}")));
            }
            Ok(json!({"message":"Application ended"}))
        })
    }
}

pub fn catalogue(ctx: &Context) -> Result<Value> {
    #[cfg(target_os = "linux")]
    {
        crate::linux::catalogue(ctx)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = ctx;
        Ok(
            json!({"runners":[],"families":[],"plugins":[],"installers":[
        {"id":"steam","name":"Steam","description":"Install the native client from Valve, then add it to your library.","url":"https://store.steampowered.com/about/"},
        {"id":"epic","name":"Epic Games","description":"Official platform installer","url":"https://store.epicgames.com/download"},
        {"id":"gog","name":"GOG Galaxy","description":"Official platform installer","url":"https://www.gog.com/galaxy"},
        {"id":"battle-net","name":"Battle.net","description":"Official platform installer","url":"https://www.blizzard.com/apps/battle.net/desktop"},
        {"id":"ea","name":"EA App","description":"Official platform installer","url":"https://www.ea.com/ea-app"},
        {"id":"ubisoft","name":"Ubisoft Connect","description":"Official platform installer","url":"https://ubisoftconnect.com"},
        {"id":"amazon","name":"Amazon Games","description":"Official platform installer","url":"https://gaming.amazon.com/amazon-games-app"},
        {"id":"rockstar","name":"Rockstar Games","description":"Official platform installer","url":"https://www.rockstargames.com/rockstar-games-launcher"},
        {"id":"discord","name":"Discord","description":"Official platform installer","url":"https://discord.com/download"}
    ],"compatibility_note":"Linux compatibility runners and Wine prefixes are specific to Linux. Use native vendor installers on this platform; vendor support varies."}),
        )
    }
}

pub fn releases(family: &str) -> Result<Value> {
    #[cfg(target_os = "linux")]
    {
        crate::linux::releases(family)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = family;
        Err(Failure::unsupported())
    }
}
pub fn work(ctx: &Context, kind: &str, payload: &Value, job: &Job) -> Result<Value> {
    if kind == "cover" {
        let id = payload["id"]
            .as_str()
            .ok_or_else(|| Failure::validation("Missing game id"))?;
        let original = crate::api::game(ctx, id)?;
        job.progress(-1.0);
        let hit = gamehandler_core::covers::fetch_cover(
            &http::UreqClient,
            &original.name,
            id,
            Some(Path::new(&original.exe_path)),
            &ctx.data.join("covers"),
            std::time::Duration::from_secs(30),
        )
        .map_err(error)?;
        if job.cancelled() {
            return Err(error("Cancelled"));
        }
        crate::api::update_game(ctx, id, |game| {
            game.cover_path = hit.cover_path.to_string_lossy().into_owned();
            game.steam_appid = hit.appid;
            if game.category == "Uncategorized" {
                game.category = hit.category.clone();
            }
        })?;
        return Ok(json!({"message": format!("Artwork fetched from {}", hit.origin_label())}));
    }
    #[cfg(target_os = "linux")]
    {
        crate::linux::work(ctx, kind, payload, job)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (ctx, kind, payload, job);
        Err(Failure::unsupported())
    }
}
pub fn remove_runner(ctx: &Context, id: &str) -> Result<Value> {
    #[cfg(target_os = "linux")]
    {
        gamehandler_core::runners::proton::uninstall(&ctx.data.join("runners"), id)
            .map_err(error)?;
        Ok(Value::Null)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (ctx, id);
        Err(Failure::unsupported())
    }
}
pub fn prefix_tool(ctx: &Context, g: &Game, tool: &str) -> Result<Value> {
    if !matches!(tool, "winecfg" | "winetricks") {
        return Err(Failure::validation("Unknown prefix tool"));
    }
    #[cfg(target_os = "linux")]
    {
        crate::linux::prefix_tool(ctx, g, tool)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (ctx, g);
        Err(Failure::unsupported())
    }
}
/// Validate a bounded snapshot before any existing cover is moved aside.
/// Copy that snapshot, not the source again: a file edited during import cannot
/// replace the validated bytes with an unreadable image.
pub fn copy_cover(ctx: &Context, id: &str, path: &Path) -> Result<std::path::PathBuf> {
    use std::io::{Read, Write};
    const MAX_BYTES: u64 = 32 * 1024 * 1024;
    let file = std::fs::File::open(path).map_err(error)?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Failure::validation(
            "Cover images must be smaller than 32 MiB",
        ));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(error)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|_| Failure::validation(
        "Choose a readable PNG, JPEG, GIF, WebP or ICO cover image (maximum 16384 pixels per side)"
    ))?;
    let directory = ctx.data.join("covers");
    std::fs::create_dir_all(&directory).map_err(error)?;
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
    let mut snapshot = tempfile::Builder::new()
        .prefix(".cover-import-")
        .suffix(&format!(".{extension}"))
        .tempfile_in(&directory)
        .map_err(error)?;
    snapshot.write_all(&bytes).map_err(error)?;
    gamehandler_core::covers::copy_custom_cover(snapshot.path(), id, &directory).map_err(error)
}

pub fn import_cover(ctx: &Context, id: &str, path: &Path) -> Result<Value> {
    let _ = crate::api::game(ctx, id)?;
    let target = copy_cover(ctx, id, path)?;
    let game = crate::api::update_game(ctx, id, |game| {
        game.cover_path = target.to_string_lossy().into_owned()
    })?;
    Ok(json!(game))
}
pub fn shortcut(ctx: &Context, g: &Game, executable: &str) -> Result<Value> {
    if g.id.is_empty()
        || !g
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(Failure::validation("Invalid shortcut game id"));
    }
    if !Path::new(executable).is_file() {
        return Err(Failure::validation(
            "The bundled command-line launcher is missing",
        ));
    }
    #[cfg(target_os = "linux")]
    {
        let flatpak = std::env::var_os("FLATPAK_ID").is_some();
        let launcher = if flatpak {
            "flatpak run --command=gamehandler-cli com.goshapps.GameHandler".to_string()
        } else {
            // GLib validates argv[0] before expanding desktop field codes. A
            // literal '%' in the bundle path would be checked as '%%' and the
            // entry rejected. Keep argv[0] stable; env forwards the decoded path
            // as an argument without introducing a shell.
            format!("/usr/bin/env -- {}", desktop_argument(executable))
        };
        let command = format!(
            "{launcher} --config-dir {} --data-dir {} --launch {}",
            desktop_argument(&ctx.config.to_string_lossy()),
            desktop_argument(&ctx.data.to_string_lossy()),
            g.id
        );
        // Flatpak's XDG_DATA_HOME is sandbox-private; its existing manifest grants
        // the host application directory explicitly. Native sessions use XDG.
        let destination = if flatpak {
            None
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(std::path::PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|p| p.join("applications"))
        };
        Ok(json!(
            gamehandler_core::runners::desktop::create_desktop_shortcut(
                g,
                &command,
                destination.as_deref()
            )?
        ))
    }

    #[cfg(target_os = "windows")]
    {
        // User values are process environment data, never PowerShell source.
        let script = r#"$ErrorActionPreference='Stop'; $desktop=[Environment]::GetFolderPath('Desktop'); $path=Join-Path $desktop ('GameHandler-'+$env:GH_SHORTCUT_ID+'.lnk'); $link=(New-Object -ComObject WScript.Shell).CreateShortcut($path); $link.TargetPath=$env:GH_SHORTCUT_EXE; $link.Arguments=$env:GH_SHORTCUT_ARGUMENTS; $link.Description=$env:GH_SHORTCUT_NAME; $link.Save(); Write-Output $path"#;
        let result = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("GH_SHORTCUT_ID", &g.id)
            .env(
                "GH_SHORTCUT_ARGUMENTS",
                format!(
                    "--config-dir {} --data-dir {} --launch {}",
                    windows_arguments::quote(&ctx.config.to_string_lossy()),
                    windows_arguments::quote(&ctx.data.to_string_lossy()),
                    g.id
                ),
            )
            .env("GH_SHORTCUT_EXE", executable)
            .env("GH_SHORTCUT_NAME", &g.name)
            .output()?;
        if !result.status.success() {
            return Err(error(String::from_utf8_lossy(&result.stderr)));
        }
        Ok(json!(String::from_utf8_lossy(&result.stdout).trim()))
    }
    #[cfg(target_os = "macos")]
    {
        let home = gamehandler_core::paths::home_dir();
        let app = home
            .join("Desktop")
            .join(format!("GameHandler-{}.app", g.id));
        let macos = app.join("Contents/MacOS");
        std::fs::create_dir_all(&macos)?;
        std::fs::copy(executable, macos.join("gamehandler-cli"))?;
        std::fs::write(macos.join("gamehandler-launch.json"), json!({"context":{"config":ctx.config,"data":ctx.data},"operation":"launch","arguments":{"id":g.id}}).to_string())?;
        let name = g
            .name
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        std::fs::write(
            app.join("Contents/Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleExecutable</key><string>gamehandler-cli</string><key>CFBundleName</key><string>{name}</string><key>CFBundleIdentifier</key><string>com.goshapps.GameHandler.launcher.{}</string><key>CFBundlePackageType</key><string>APPL</string><key>LSUIElement</key><true/></dict></plist>"#,
                g.id
            ),
        )?;
        Ok(json!(app))
    }
}
pub fn plugin_command(id: &str) -> Result<Value> {
    #[cfg(target_os = "linux")]
    {
        crate::linux::plugin_command(id)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = id;
        Err(Failure::unsupported())
    }
}
pub fn complete_install(ctx: &Context, data: &Value, path: &Path) -> Result<Value> {
    #[cfg(target_os = "linux")]
    {
        crate::linux::complete_install(ctx, data, path)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (ctx, data, path);
        Err(Failure::unsupported())
    }
}

#[cfg(target_os = "linux")]
fn desktop_argument(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`")
            .replace('$', "\\$")
    )
}
