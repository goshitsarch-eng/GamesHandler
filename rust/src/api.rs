use gamehandler_core::{
    models::{Game, Library},
    settings::Settings,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Mutex};

static STORE: Mutex<()> = Mutex::new(());

#[derive(Clone, Deserialize)]
pub struct Context {
    pub config: PathBuf,
    pub data: PathBuf,
}

#[derive(Deserialize)]
struct Request {
    context: Context,
    #[serde(flatten)]
    command: Operation,
}

#[derive(Deserialize)]
#[serde(tag = "operation", content = "arguments", rename_all = "snake_case")]
enum Operation {
    NewGame,
    Snapshot {
        #[serde(default)]
        query: String,
        #[serde(default = "all")]
        category: String,
        #[serde(default = "name")]
        sort: String,
    },
    SaveGame {
        game: Value,
    },
    RemoveGame {
        id: String,
    },
    SaveSettings {
        settings: Value,
    },
    Launch {
        id: String,
    },
    StartJob {
        kind: String,
        payload: Value,
    },
    Jobs,
    CancelJob {
        id: String,
    },
    DismissJob {
        id: String,
    },
    Catalogue,
    Releases {
        family: String,
    },
    RemoveRunner {
        id: String,
    },
    PrefixTool {
        id: String,
        tool: String,
    },
    PrefixPath {
        id: String,
    },
    ImportCover {
        id: String,
        path: PathBuf,
    },
    Shortcut {
        id: String,
        executable: String,
    },
    PluginCommand {
        id: String,
    },
    CompleteInstall {
        job: String,
        path: PathBuf,
    },
}
fn all() -> String {
    "All".into()
}
fn name() -> String {
    "name".into()
}

#[derive(Debug)]
pub struct Failure {
    pub kind: &'static str,
    pub message: String,
}
pub type Result<T> = std::result::Result<T, Failure>;
impl Failure {
    pub fn validation(message: impl Into<String>) -> Self {
        Self {
            kind: "validation",
            message: message.into(),
        }
    }
    #[cfg(not(target_os = "linux"))]
    pub fn unsupported() -> Self {
        Self { kind:"unsupported", message:"This compatibility operation is available on Linux. Native applications can be added directly on this platform.".into() }
    }
}
impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Self {
            kind: if e.kind() == std::io::ErrorKind::PermissionDenied {
                "permission"
            } else {
                "file"
            },
            message: e.to_string(),
        }
    }
}
pub fn error(e: impl std::fmt::Display) -> Failure {
    Failure {
        kind: "operation",
        message: e.to_string(),
    }
}

pub fn execute(bytes: &[u8]) -> Value {
    let result = serde_json::from_slice::<Request>(bytes)
        .map_err(|e| Failure::validation(e.to_string()))
        .and_then(dispatch);
    match result {
        Ok(data) => json!({"ok":true,"data":data}),
        Err(e) => json!({"ok":false,"error":{"kind":e.kind,"message":e.message}}),
    }
}

pub fn library(ctx: &Context) -> Result<Library> {
    let l = Library::new(Some(ctx.config.join("games.json")));
    if l.load_status().is_destructive_to_save_over() {
        return Err(Failure {kind:"corrupt_data",message:"The library cannot be read. Its original bytes have been preserved; repair it or restore a backup before editing.".into()});
    }
    Ok(l)
}

fn persistence_lock(ctx: &Context) -> Result<std::fs::File> {
    std::fs::create_dir_all(&ctx.config)?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(ctx.config.join(".write.lock"))?;
    lock.lock()?;
    Ok(lock)
}

pub fn store_game(ctx: &Context, mut game: Game) -> Result<()> {
    let _guard = STORE.lock().map_err(error)?;
    let _file = persistence_lock(ctx)?;
    let mut l = library(ctx)?;
    if game.id.is_empty() {
        game.id = gamehandler_core::models::new_id();
    }
    if let Some(old) = l.get(&game.id) {
        game.added = old.added;
        game.last_played = old.last_played;
    }
    l.add(game).map_err(error)
}

pub(crate) fn update_game(ctx: &Context, id: &str, change: impl FnOnce(&mut Game)) -> Result<Game> {
    let _guard = STORE.lock().map_err(error)?;
    let _file = persistence_lock(ctx)?;
    let mut library = library(ctx)?;
    let mut game = library
        .get(id)
        .cloned()
        .ok_or_else(|| Failure::validation("The game no longer exists"))?;
    change(&mut game);
    library.add(game.clone()).map_err(error)?;
    Ok(game)
}

pub(crate) fn validate_path_id(id: &str) -> Result<()> {
    if id.is_empty()
        || matches!(id, "." | "..")
        || id.chars().any(|c| matches!(c, '/' | '\\' | ':' | '\0'))
    {
        return Err(Failure::validation(
            "The game id cannot be used as a safe directory name; its original record has been preserved",
        ));
    }
    Ok(())
}

pub fn game(ctx: &Context, id: &str) -> Result<Game> {
    library(ctx)?
        .get(id)
        .cloned()
        .ok_or_else(|| Failure::validation("The game no longer exists"))
}

fn dispatch(request: Request) -> Result<Value> {
    let ctx = request.context;
    if !ctx.config.is_absolute() || !ctx.data.is_absolute() {
        return Err(Failure::validation(
            "Application directories must be absolute",
        ));
    }
    match request.command {
        Operation::NewGame => {
            let s = Settings::load(Some(ctx.config.join("settings.json")));
            let settings = serde_json::to_value(s).map_err(error)?;
            let mut value = serde_json::to_value(Game::new_named("")).map_err(error)?;
            for key in [
                "mangohud",
                "gamemode",
                "prefer_sdl",
                "esync",
                "fsync",
                "dxvk",
                "vkd3d",
                "nvapi",
                "fsr",
                "battleye",
                "eac",
                "gamescope",
                "virtual_desktop",
            ] {
                if let Some(v) = settings.get(format!("default_{key}")) {
                    value[key] = v.clone();
                }
            }
            value["runner"] = settings["default_runner"].clone();
            if !cfg!(target_os = "linux") {
                value["kind"] = json!(if cfg!(target_os = "windows") {
                    "windows"
                } else {
                    "native"
                });
            }
            Ok(value)
        }
        Operation::Snapshot {
            query,
            category,
            sort,
        } => {
            let _guard = STORE.lock().map_err(error)?;
            let l = library(&ctx)?;
            let s = Settings::load(Some(ctx.config.join("settings.json")));
            Ok(
                json!({"games":l.search(&query,&category,&sort),"categories":l.categories(),"settings":s,"platform":std::env::consts::OS,"compatibility":cfg!(target_os="linux"),"version":env!("CARGO_PKG_VERSION")}),
            )
        }
        Operation::SaveGame { game: value } => {
            let fields = value
                .as_object()
                .ok_or_else(|| Failure::validation("Expected a game record"))?;
            let mut g = Game::from_dict(fields);
            if g.name.trim().is_empty() {
                return Err(Failure::validation("Enter a game name"));
            }
            let executable = std::path::Path::new(&g.exe_path);
            let app_bundle = (g.kind == "macos"
                || (g.kind == "native" && cfg!(target_os = "macos")))
                && executable.is_dir()
                && executable
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("app"));
            if !executable.is_file() && !app_bundle {
                return Err(Failure::validation(
                    "Choose an executable file or a macOS .app bundle",
                ));
            }
            if !matches!(g.kind.as_str(), "windows" | "linux" | "native" | "macos") {
                return Err(Failure::validation("Unknown game type"));
            }
            if g.id.is_empty() {
                g.id = gamehandler_core::models::new_id();
            }
            if !g.cover_path.is_empty()
                && !PathBuf::from(&g.cover_path).starts_with(ctx.data.join("covers"))
            {
                let _ = library(&ctx)?;
                g.cover_path =
                    crate::native::copy_cover(&ctx, &g.id, std::path::Path::new(&g.cover_path))?
                        .to_string_lossy()
                        .into_owned();
            }
            store_game(&ctx, g.clone())?;
            Ok(json!(g))
        }
        Operation::RemoveGame { id } => {
            let _guard = STORE.lock().map_err(error)?;
            let _file = persistence_lock(&ctx)?;
            library(&ctx)?.remove(&id).map_err(error)?;
            Ok(Value::Null)
        }
        Operation::SaveSettings { settings } => {
            let _guard = STORE.lock().map_err(error)?;
            let _file = persistence_lock(&ctx)?;
            let path = ctx.config.join("settings.json");
            if path.exists() {
                let bytes = std::fs::read(&path)?;
                let source = std::str::from_utf8(&bytes).map_err(error)?;
                if !gamehandler_core::json::parse_lenient(source)
                    .map_err(error)?
                    .is_object()
                {
                    return Err(Failure {
                        kind: "corrupt_data",
                        message:
                            "Settings are not an object; the original file has been preserved."
                                .into(),
                    });
                }
            }
            let s = Settings::from_dict(
                settings
                    .as_object()
                    .ok_or_else(|| Failure::validation("Expected settings"))?,
            );
            s.save_to(path).map_err(error)?;
            Ok(json!(s))
        }
        Operation::Launch { id } => {
            let g = game(&ctx, &id)?;
            crate::native::launch(&ctx, &g)?;
            let _guard = STORE.lock().map_err(error)?;
            let _file = persistence_lock(&ctx)?;
            library(&ctx)?.mark_played(&id).map_err(error)?;
            Ok(Value::Null)
        }
        Operation::StartJob { kind, payload } => crate::jobs::start(ctx, kind, payload),
        Operation::Jobs => Ok(crate::jobs::snapshot()),
        Operation::CancelJob { id } => {
            crate::jobs::cancel(&id)?;
            Ok(Value::Null)
        }
        Operation::DismissJob { id } => {
            crate::jobs::dismiss(&id)?;
            Ok(Value::Null)
        }
        Operation::Catalogue => {
            let mut value = crate::native::catalogue(&ctx)?;
            value["credits"]=json!(gamehandler_core::credits::sections().iter().map(|s|json!({"title":s.title,"summary":s.summary,"entries":s.entries.iter().map(|c|json!({"name":c.name,"description":c.role,"license":c.license,"url":c.url,"authors":c.authors})).collect::<Vec<_>>()})).collect::<Vec<_>>());
            Ok(value)
        }
        Operation::Releases { family } => crate::native::releases(&family),
        Operation::RemoveRunner { id } => crate::native::remove_runner(&ctx, &id),
        Operation::PrefixTool { id, tool } => {
            crate::native::prefix_tool(&ctx, &game(&ctx, &id)?, &tool)
        }
        Operation::PrefixPath { id } => {
            let g = game(&ctx, &id)?;
            Ok(json!(if g.prefix_path.is_empty() {
                validate_path_id(&g.id)?;
                ctx.data.join("prefixes").join(g.id)
            } else {
                PathBuf::from(g.prefix_path)
            }))
        }
        Operation::ImportCover { id, path } => crate::native::import_cover(&ctx, &id, &path),
        Operation::Shortcut { id, executable } => {
            crate::native::shortcut(&ctx, &game(&ctx, &id)?, &executable)
        }
        Operation::PluginCommand { id } => crate::native::plugin_command(&id),
        Operation::CompleteInstall { job, path } => {
            crate::jobs::complete_install(&ctx, &job, &path)
        }
    }
}
