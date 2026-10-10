use crate::api::{Context, Failure, Result, error};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

pub struct Job {
    pub cancel: AtomicBool,
    pub state: Mutex<Value>,
}
pub static JOBS: LazyLock<Mutex<BTreeMap<String, Arc<Job>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
impl Job {
    pub fn progress(&self, fraction: f64) {
        if let Ok(mut s) = self.state.lock() {
            s["progress"] = json!(fraction);
        }
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}
pub fn start(ctx: Context, kind: String, payload: Value) -> Result<Value> {
    if !matches!(kind.as_str(), "runner" | "cover" | "installer") {
        return Err(Failure::validation("Unknown background operation"));
    }
    let id = gamehandler_core::models::new_id();
    let job = Arc::new(Job {
        cancel: AtomicBool::new(false),
        state: Mutex::new(
            json!({"id":id,"kind":kind,"payload":payload,"status":"running","progress":-1.0}),
        ),
    });
    let mut registry = JOBS.lock().map_err(error)?;
    if registry.values().any(|j| {
        j.state
            .lock()
            .is_ok_and(|s| s["status"] == "running" && s["kind"] == kind)
    }) {
        return Err(Failure::validation(
            "An operation of this kind is already running",
        ));
    }
    registry.insert(id.clone(), job.clone());
    drop(registry);
    thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::native::work(&ctx, &kind, &payload, &job)
        }))
        .unwrap_or_else(|_| Err(error("Background operation failed unexpectedly")));
        if let Ok(mut state) = job.state.lock() {
            match result {
                Ok(value) => {
                    state["status"] =
                        json!(if value.get("needs_executable") == Some(&json!(true)) {
                            "needs_executable"
                        } else {
                            "complete"
                        });
                    state["result"] = value;
                }
                Err(e) => {
                    state["status"] = json!(if job.cancelled() {
                        "cancelled"
                    } else {
                        "failed"
                    });
                    state["error"] = json!({"kind":e.kind,"message":e.message});
                }
            }
        }
    });
    Ok(json!({"id":id}))
}
pub fn snapshot() -> Value {
    match JOBS.lock() {
        Ok(reg) => json!(
            reg.values()
                .filter_map(|j| j.state.lock().ok().map(|s| s.clone()))
                .collect::<Vec<_>>()
        ),
        Err(_) => json!([]),
    }
}
pub fn cancel(id: &str) -> Result<()> {
    let jobs = JOBS.lock().map_err(error)?;
    let j = jobs
        .get(id)
        .ok_or_else(|| Failure::validation("Operation not found"))?;
    if j.state.lock().map_err(error)?["kind"] == "launch" {
        return Err(Failure::validation(
            "Launched games run independently; close them in their own window",
        ));
    }
    j.cancel.store(true, Ordering::Relaxed);
    Ok(())
}
pub fn dismiss(id: &str) -> Result<()> {
    let mut jobs = JOBS.lock().map_err(error)?;
    if jobs
        .get(id)
        .is_some_and(|j| j.state.lock().is_ok_and(|s| s["status"] == "running"))
    {
        return Err(Failure::validation(
            "Cancel and wait for the operation before dismissing it",
        ));
    }
    jobs.remove(id);
    Ok(())
}
pub fn complete_install(ctx: &Context, id: &str, path: &Path) -> Result<Value> {
    if !path.is_file() {
        return Err(Failure::validation("Choose an existing executable"));
    }
    let j = JOBS
        .lock()
        .map_err(error)?
        .get(id)
        .cloned()
        .ok_or_else(|| Failure::validation("Operation not found"))?;
    let mut s = j.state.lock().map_err(error)?;
    if s["status"] != "needs_executable" {
        return Err(Failure::validation(
            "This installer is not waiting for an executable",
        ));
    }
    let value = crate::native::complete_install(ctx, &s["result"], path)?;
    s["status"] = json!("complete");
    s["result"] = value.clone();
    Ok(value)
}

/// Observe a launched process without giving the UI ownership of its lifetime.
pub fn observe_launch(
    id: String,
    observe: impl FnOnce() -> Result<Value> + Send + 'static,
) -> Result<()> {
    let key = gamehandler_core::models::new_id();
    let job = Arc::new(Job {
        cancel: AtomicBool::new(false),
        state: Mutex::new(
            json!({"id":key,"kind":"launch","payload":{"id":id},"status":"running","progress":-1.0,"result":{"message":"Game is running"}}),
        ),
    });
    JOBS.lock().map_err(error)?.insert(key, job.clone());
    thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(observe))
            .unwrap_or_else(|_| Err(error("Launch monitor failed unexpectedly")));
        if let Ok(mut state) = job.state.lock() {
            match result {
                Ok(value) => {
                    state["status"] = json!("complete");
                    state["result"] = value;
                }
                Err(failure) => {
                    state["status"] = json!("failed");
                    state["error"] = json!({"kind":failure.kind,"message":failure.message});
                }
            }
        }
    });
    Ok(())
}
