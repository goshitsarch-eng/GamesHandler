use crate::native::http;
use crate::{
    api::{Context, Failure, Result, error},
    jobs::Job,
};
use gamehandler_core::{
    models::Game,
    netpaths::NetpathsShares,
    paths::Env,
    runners::{LaunchEnv, RunnerManager, SystemLaunchEnv},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

struct Environment<'a>(&'a Context);
impl Env for Environment<'_> {
    fn var(&self, key: &str) -> Option<String> {
        match key {
            "GAMEHANDLER_CONFIG_HOME" => Some(self.0.config.to_string_lossy().into_owned()),
            "GAMEHANDLER_DATA_HOME" => Some(self.0.data.to_string_lossy().into_owned()),
            "GAMEHANDLER_AUTHENTICODE_ROOT" | "GAMEHANDLER_DXVK_ROOT" => {
                if let Some(value) = Env::var(&SystemLaunchEnv, key) {
                    return Some(value);
                }
                let name = if key == "GAMEHANDLER_AUTHENTICODE_ROOT" {
                    "microsoft-identity-verification-root-ca-2020.pem"
                } else {
                    "dxvk"
                };
                let bundled = std::env::current_exe()
                    .ok()?
                    .parent()?
                    .join("share/gamehandler")
                    .join(name);
                bundled
                    .exists()
                    .then(|| bundled.to_string_lossy().into_owned())
            }
            _ => Env::var(&SystemLaunchEnv, key),
        }
    }
}
impl LaunchEnv for Environment<'_> {
    fn which(&self, key: &str) -> Option<PathBuf> {
        SystemLaunchEnv.which(key)
    }
    fn environ(&self) -> BTreeMap<String, String> {
        SystemLaunchEnv.environ()
    }
    fn var(&self, key: &str) -> Option<String> {
        Env::var(self, key)
    }
    fn anticheat_runtime(&self, kind: &str, roots: &[PathBuf]) -> Option<String> {
        SystemLaunchEnv.anticheat_runtime(kind, roots)
    }
}
pub fn launch(ctx: &Context, g: &Game) -> Result<()> {
    let mut g = g.clone();
    if g.kind == "windows" && g.prefix_path.is_empty() {
        crate::api::validate_path_id(&g.id)?;
    }
    if matches!(g.kind.as_str(), "native" | "macos") {
        g.kind = "linux".into();
    }
    let env = Environment(ctx);
    let manager = RunnerManager::new(&env);
    let mut launched =
        gamehandler_core::runners::launch::launch(&g, &manager, &env, &NetpathsShares::new(&env))
            .map_err(error)?;
    // Detect immediate launch failures before reporting success or recording play.
    if let Some(reason) = launched
        .failure(Duration::from_millis(200))
        .map_err(error)?
    {
        return Err(error(reason));
    }
    crate::jobs::observe_launch(g.id.clone(), move || {
        launched.child_mut().wait()?;
        if let Some(reason) = launched.failure(Duration::ZERO)? {
            return Err(error(reason));
        }
        Ok(json!({"message":"Game ended"}))
    })
}
pub fn catalogue(ctx: &Context) -> Result<Value> {
    use gamehandler_core::{
        installers::installers,
        plugins::{SystemPluginEnv, plugin_rows},
        runners::families::RUNNER_FAMILIES,
    };
    let env = Environment(ctx);
    let manager = RunnerManager::new(&env);
    let scan = manager.scan_installed();
    if let Some(problem) = scan.problem() {
        return Err(error(problem));
    }
    Ok(json!({
        "runners":manager.all_runners(&env).iter().map(|r|json!({"id":r.id(),"name":r.name(),"available":r.is_available(),"family":r.family_id()})).collect::<Vec<_>>(),
        "families":RUNNER_FAMILIES.iter().map(|f|json!({"id":f.id,"name":f.name,"description":f.description,"guide":f.when_to_use,"url":f.homepage()})).collect::<Vec<_>>(),
        "installers":installers().iter().map(|i|json!({"id":i.id,"name":i.name,"description":i.description,"category":i.category,"notes":i.notes,"url":i.download_url})).collect::<Vec<_>>(),
        "plugins":plugin_rows(&SystemPluginEnv).iter().map(|p|json!({"id":p.plugin_id,"name":p.name,"description":p.subtitle,"status":p.state.as_str()})).collect::<Vec<_>>()
    }))
}
pub fn releases(family: &str) -> Result<Value> {
    let releases = gamehandler_core::runners::proton::fetch_available(
        &http::UreqClient,
        Some(family),
        20,
        Duration::from_secs(30),
    )
    .map_err(error)?;
    // The old catalogue can choose ARM assets on x86_64. Refuse mismatched binaries.
    let releases = releases
        .into_iter()
        .filter(|r| {
            let n = r.download_url.to_lowercase();
            if std::env::consts::ARCH == "x86_64" {
                !n.contains("aarch64") && !n.contains("arm64")
            } else {
                n.contains("aarch64") || n.contains("arm64")
            }
        })
        .collect::<Vec<_>>();
    Ok(json!(releases))
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .ok_or_else(|| Failure::validation(format!("Missing {key}")))
}
pub fn work(ctx: &Context, kind: &str, payload: &Value, job: &Job) -> Result<Value> {
    match kind {
        "runner" => {
            let family = text(payload, "family")?;
            let tag = text(payload, "tag")?;
            let entries = releases(family)?;
            let r: gamehandler_core::runners::families::ReleaseInfo = serde_json::from_value(
                entries
                    .as_array()
                    .and_then(|r| r.iter().find(|r| r["tag"] == tag))
                    .cloned()
                    .ok_or_else(|| {
                        Failure::validation(
                            "This release is no longer available for this architecture",
                        )
                    })?,
            )
            .map_err(error)?;
            let path = gamehandler_core::runners::proton::install(
                &http::UreqClient,
                &ctx.data.join("runners"),
                &r,
                &|p| job.progress(p as f64),
                Duration::from_secs(600),
                &|| job.cancelled(),
            )
            .map_err(error)?;
            Ok(json!({"path":path,"message":format!("Installed {}",r.name)}))
        }
        "installer" => install(ctx, payload, job),
        _ => Err(Failure::validation("Unknown background operation")),
    }
}
fn install(ctx: &Context, payload: &Value, job: &Job) -> Result<Value> {
    use gamehandler_core::installers::*;
    let env = Environment(ctx);
    let installer = installer_by_id(text(payload, "id")?).map_err(error)?;
    let runner_id = text(payload, "runner")?;
    let manager = RunnerManager::new(&env);
    let runner = manager.get(runner_id, &env);
    if !runner.is_available() {
        return Err(Failure::validation(
            "Download a runner or install Wine first",
        ));
    }
    let game_id = gamehandler_core::models::new_id();
    let prefix = prepare_prefix_in(&game_id, &env)?;
    let archive = download_installer_with(
        installer,
        &ctx.data.join("downloads"),
        Some(&|p| job.progress(p)),
        Duration::from_secs(120),
        &http::UreqClient,
        &env,
        &|| job.cancelled(),
    )
    .map_err(error)?;
    job.progress(-1.0);
    let command = build_installer_command(runner.as_ref(), &prefix, installer, &archive, &env)
        .map_err(error)?;
    let mut child = Command::new(&command.argv[0])
        .args(&command.argv[1..])
        .env_clear()
        .envs(&command.env)
        .current_dir(archive.parent().unwrap_or(Path::new(".")))
        .spawn()?;
    loop {
        if job.cancelled() {
            child.kill()?;
            child.wait()?;
            return Err(error("Cancelled; the prefix was kept"));
        }
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                return Err(error(format!(
                    "Vendor installer exited with {status}; the prefix was kept"
                )));
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let found = wait_for_installer(
        runner.as_ref(),
        &command.env,
        &prefix,
        installer.expected_exe,
        &SystemClock,
        &|r, e, s| wait_for_prefix_idle(r, e, s, &env),
        &|| job.cancelled(),
    );
    if job.cancelled() {
        return Err(error("Cancelled; the prefix was kept"));
    }
    let pending = json!({"installer":installer.id,"runner":runner_id,"prefix":prefix,"game_id":game_id,"needs_executable":found.is_none()});
    match found {
        Some(path) => complete_install(ctx, &pending, &path),
        None => Ok(pending),
    }
}
pub fn complete_install(ctx: &Context, data: &Value, path: &Path) -> Result<Value> {
    use gamehandler_core::installers::*;
    let installer = installer_by_id(text(data, "installer")?).map_err(error)?;
    let g = game_from_install(
        installer,
        path,
        Path::new(text(data, "prefix")?),
        text(data, "runner")?,
        Some(text(data, "game_id")?),
    );
    crate::api::store_game(ctx, g.clone())?;
    Ok(json!({"game":g,"message":format!("{} added to your library",installer.name)}))
}
pub fn prefix_tool(ctx: &Context, g: &Game, tool: &str) -> Result<Value> {
    if g.prefix_path.is_empty() {
        crate::api::validate_path_id(&g.id)?;
    }
    let env = Environment(ctx);
    let manager = RunnerManager::new(&env);
    let command =
        gamehandler_core::runners::launch::tool_command(g, &manager, tool, &env).map_err(error)?;
    let mut child = Command::new(&command.argv[0])
        .args(&command.argv[1..])
        .env_clear()
        .envs(&command.env)
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(Value::Null)
}
pub fn plugin_command(id: &str) -> Result<Value> {
    use gamehandler_core::plugins::*;
    let plugin = plugin_by_id(id).ok_or_else(|| Failure::validation("Unknown helper"))?;
    let command = install_command(plugin, None, &SystemPluginEnv).map_err(error)?;
    Ok(json!({"argv":command,"display":format_command(&command)}))
}
