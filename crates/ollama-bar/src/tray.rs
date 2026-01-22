//! Tray icon implementation using tray-icon crate

use crate::state::AppState;
use anyhow::Result;
use llm_core::{OllamaStatus, TailscaleStatus};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu, CheckMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

// Menu item IDs
const ID_START: &str = "start";
const ID_STOP: &str = "stop";
const ID_RESTART: &str = "restart";
const ID_TOGGLE_TAILSCALE: &str = "toggle_tailscale";
const ID_COPY_URL: &str = "copy_url";
const ID_PULL_MODEL: &str = "pull_model";
const ID_VIEW_LOGS: &str = "view_logs";
const ID_SETTINGS: &str = "settings";
const ID_QUIT: &str = "quit";

// Track last model load error
use std::sync::Mutex;
static LAST_MODEL_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub struct TrayManager {
    pub state: AppState,
    tray_icon: Option<TrayIcon>,
}

impl TrayManager {
    pub fn new(state: AppState) -> Result<Self> {
        Ok(Self {
            state,
            tray_icon: None,
        })
    }

    pub fn create_tray(&mut self) -> Result<()> {
        let menu = self.build_menu()?;

        // Create tray icon with a simple circle
        let tray = TrayIconBuilder::new()
            .with_tooltip("OllamaBar")
            .with_title("○")
            .with_menu(Box::new(menu))
            .build()?;

        self.tray_icon = Some(tray);
        tracing::info!("Tray icon created");
        Ok(())
    }

    fn build_menu(&self) -> Result<Menu> {
        let menu = Menu::new();

        let status = self.state.ollama_status();
        let ts_status = self.state.tailscale_status();
        let sharing = self.state.tailscale_sharing();
        let models = self.state.available_models();
        let current_model = self.state.current_model();

        tracing::debug!(
            "Building menu: status={:?}, current_model={:?}, models_count={}",
            status, current_model, models.len()
        );

        // Status section
        let status_text = match status {
            OllamaStatus::Running => "● Ollama Running",
            OllamaStatus::Starting => "◐ Ollama Starting...",
            OllamaStatus::Stopped => "○ Ollama Stopped",
            OllamaStatus::Error => "⊘ Ollama Error",
        };
        let status_item = MenuItem::new(status_text, false, None);
        menu.append(&status_item)?;

        // Current model display (when running) or last model (when stopped)
        let last_model = self.state.last_model();
        if let Some(model) = &current_model {
            let model_text = format!("  Model: {}", model);
            let model_item = MenuItem::new(model_text, false, None);
            menu.append(&model_item)?;
        } else if status == OllamaStatus::Stopped {
            if let Some(model) = &last_model {
                let model_text = format!("  Last: {}", model);
                let model_item = MenuItem::new(model_text, false, None);
                menu.append(&model_item)?;
            }
        }

        // Memory info
        let (used, total) = self.state.memory_info();
        let mem_text = format!("  Memory: {:.1} / {:.0} GB", used, total);
        let mem_item = MenuItem::new(mem_text, false, None);
        menu.append(&mem_item)?;

        menu.append(&PredefinedMenuItem::separator())?;

        // Start/Stop actions
        if status == OllamaStatus::Running {
            let stop_item = MenuItem::with_id(ID_STOP, "Stop Ollama", true, None);
            menu.append(&stop_item)?;
        } else {
            let start_item = MenuItem::with_id(ID_START, "Start Ollama", true, None);
            menu.append(&start_item)?;

            // Offer to start with last model
            if let Some(model) = &last_model {
                let start_with_item = MenuItem::with_id(
                    format!("start_with:{}", model),
                    format!("Start with {}", model),
                    true,
                    None,
                );
                menu.append(&start_with_item)?;
            }
        }

        let restart_item = MenuItem::with_id(ID_RESTART, "Restart", status == OllamaStatus::Running, None);
        menu.append(&restart_item)?;

        menu.append(&PredefinedMenuItem::separator())?;

        // Check for last model error
        let last_error = LAST_MODEL_ERROR.lock().ok().and_then(|e| e.clone());
        if let Some(err) = &last_error {
            let err_item = MenuItem::new(format!("⚠ Error: {}", err), false, None);
            menu.append(&err_item)?;

            // Extract model name from error and offer repull
            if let Some(model_name) = err.split(':').next() {
                let repull_item = MenuItem::with_id(
                    format!("repull:{}", model_name),
                    format!("↻ Re-pull {}", model_name),
                    true,
                    None,
                );
                menu.append(&repull_item)?;
            }
            menu.append(&PredefinedMenuItem::separator())?;
        }

        // Model switching submenu
        if !models.is_empty() {
            let model_submenu = Submenu::new("Switch Model", true);
            for model in &models {
                let is_current = current_model.as_ref() == Some(model);
                let item = CheckMenuItem::with_id(
                    format!("model:{}", model),
                    model,
                    true,
                    is_current,
                    None,
                );
                model_submenu.append(&item)?;
            }
            menu.append(&model_submenu)?;
        } else {
            let no_models = MenuItem::new("No models available", false, None);
            menu.append(&no_models)?;
        }

        menu.append(&PredefinedMenuItem::separator())?;

        // Tailscale section
        let ts_text = match ts_status {
            TailscaleStatus::Connected => "Tailscale: Connected",
            TailscaleStatus::Disconnected => "Tailscale: Disconnected",
            TailscaleStatus::NotInstalled => "Tailscale: Not Installed",
        };
        let ts_item = MenuItem::new(ts_text, false, None);
        menu.append(&ts_item)?;

        if ts_status == TailscaleStatus::Connected {
            let share_text = if sharing {
                "☑ Share via Tailscale"
            } else {
                "☐ Share via Tailscale"
            };
            let share_item = MenuItem::with_id(ID_TOGGLE_TAILSCALE, share_text, true, None);
            menu.append(&share_item)?;

            if sharing {
                if let Some(url) = self.state.tailscale_serve_url() {
                    let url_text = format!("  {}  [Copy]", url);
                    let url_item = MenuItem::with_id(ID_COPY_URL, url_text, true, None);
                    menu.append(&url_item)?;
                }
            }
        }

        menu.append(&PredefinedMenuItem::separator())?;

        // Footer
        let pull_item = MenuItem::with_id(ID_PULL_MODEL, "Pull Model...", true, None);
        menu.append(&pull_item)?;

        let logs_item = MenuItem::with_id(ID_VIEW_LOGS, "View Logs", true, None);
        menu.append(&logs_item)?;

        let settings_item = MenuItem::with_id(ID_SETTINGS, "Settings...", true, None);
        menu.append(&settings_item)?;

        menu.append(&PredefinedMenuItem::separator())?;

        // Version
        let version = env!("CARGO_PKG_VERSION");
        let version_item = MenuItem::new(format!("OllamaBar v{}", version), false, None);
        menu.append(&version_item)?;

        let quit_item = MenuItem::with_id(ID_QUIT, "Quit", true, None);
        menu.append(&quit_item)?;

        Ok(menu)
    }

    pub fn update_menu(&mut self) -> Result<()> {
        if let Some(tray) = &self.tray_icon {
            let menu = self.build_menu()?;
            tray.set_menu(Some(Box::new(menu)));
        }
        Ok(())
    }

    pub fn update_icon(&self) {
        if let Some(tray) = &self.tray_icon {
            let status = self.state.ollama_status();
            let sharing = self.state.tailscale_sharing();

            let icon = match (status, sharing) {
                (OllamaStatus::Running, true) => "●↗",
                (OllamaStatus::Running, false) => "●",
                (OllamaStatus::Starting, _) => "◐",
                (OllamaStatus::Stopped, _) => "○",
                (OllamaStatus::Error, _) => "⊘",
            };

            tray.set_title(Some(icon));
        }
    }

    /// Handle a single menu event, returns true if should quit
    pub fn handle_event(&self, event: &MenuEvent) -> bool {
        let id_str = event.id.0.as_str();
        tracing::info!("=== MENU EVENT ID: '{}' ===", id_str);

        match id_str {
            ID_START => self.handle_start(),
            ID_STOP => self.handle_stop(),
            ID_RESTART => self.handle_restart(),
            ID_TOGGLE_TAILSCALE => self.handle_toggle_tailscale(),
            ID_COPY_URL => self.handle_copy_url(),
            ID_PULL_MODEL => self.handle_pull_model(),
            ID_VIEW_LOGS => self.handle_view_logs(),
            ID_SETTINGS => self.handle_settings(),
            ID_QUIT => return true,
            id if id.starts_with("model:") => {
                let model = id.strip_prefix("model:").unwrap();
                self.handle_switch_model(model);
            }
            id if id.starts_with("repull:") => {
                let model = id.strip_prefix("repull:").unwrap();
                self.handle_repull_model(model);
            }
            id if id.starts_with("start_with:") => {
                let model = id.strip_prefix("start_with:").unwrap();
                self.handle_start_with_model(model);
            }
            _ => {}
        }

        false
    }

    fn handle_start(&self) {
        tracing::info!("Starting Ollama...");
        let state = self.state.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                if let Err(e) = state.start_ollama().await {
                    tracing::error!("Failed to start Ollama: {}", e);
                }
            });
        });
    }

    fn handle_start_with_model(&self, model: &str) {
        tracing::info!("Starting Ollama with model: {}", model);
        let state = self.state.clone();
        let model = model.to_string();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                match state.start_ollama_with_model(&model).await {
                    Ok(()) => tracing::info!("Ollama started with model: {}", model),
                    Err(e) => {
                        tracing::error!("Failed to start Ollama with model: {}", e);
                        if let Ok(mut last_err) = LAST_MODEL_ERROR.lock() {
                            *last_err = Some(format!("{}: {}", model, e));
                        }
                    }
                }
            });
        });
    }

    fn handle_stop(&self) {
        tracing::info!("Stopping Ollama...");
        if let Err(e) = self.state.stop_ollama() {
            tracing::error!("Failed to stop Ollama: {}", e);
        }
    }

    fn handle_restart(&self) {
        tracing::info!("Restarting Ollama...");
        let state = self.state.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let _ = state.stop_ollama();
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if let Err(e) = state.start_ollama().await {
                    tracing::error!("Failed to restart Ollama: {}", e);
                }
            });
        });
    }

    fn handle_switch_model(&self, model: &str) {
        tracing::info!("=== SWITCH MODEL REQUESTED: '{}' ===", model);
        // Clear previous error
        if let Ok(mut err) = LAST_MODEL_ERROR.lock() {
            *err = None;
        }

        let state = self.state.clone();
        let model = model.to_string();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                tracing::info!("Loading model in background thread: {}", model);
                match state.switch_model(&model).await {
                    Ok(()) => {
                        tracing::info!("Model switch completed: {}", model);
                    }
                    Err(e) => {
                        let err_msg = format!("{}: {}", model, e);
                        tracing::error!("Failed to switch model: {}", err_msg);
                        if let Ok(mut last_err) = LAST_MODEL_ERROR.lock() {
                            *last_err = Some(err_msg);
                        }
                    }
                }
            });
        });
    }

    fn handle_repull_model(&self, model: &str) {
        tracing::info!("=== RE-PULL MODEL REQUESTED: '{}' ===", model);
        // Clear previous error
        if let Ok(mut err) = LAST_MODEL_ERROR.lock() {
            *err = None;
        }

        let state = self.state.clone();
        let model = model.to_string();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                tracing::info!("Re-pulling model: {}", model);
                match state.pull_model(&model).await {
                    Ok(()) => {
                        tracing::info!("Model re-pull completed: {}", model);
                    }
                    Err(e) => {
                        let err_msg = format!("Pull failed for {}: {}", model, e);
                        tracing::error!("{}", err_msg);
                        if let Ok(mut last_err) = LAST_MODEL_ERROR.lock() {
                            *last_err = Some(err_msg);
                        }
                    }
                }
            });
        });
    }

    fn handle_toggle_tailscale(&self) {
        tracing::info!("Toggling Tailscale sharing...");
        if let Err(e) = self.state.toggle_tailscale_sharing() {
            tracing::error!("Failed to toggle Tailscale: {}", e);
        }
    }

    fn handle_copy_url(&self) {
        if let Some(url) = self.state.tailscale_serve_url() {
            tracing::info!("Copying URL: {}", url);
            use std::io::Write;
            if let Ok(mut child) = std::process::Command::new("pbcopy")
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = stdin.write_all(url.as_bytes());
                }
                let _ = child.wait();
            }
        }
    }

    fn handle_pull_model(&self) {
        tracing::info!("Pull model requested");

        // Show macOS dialog to get model name using osascript
        let script = r#"
            set dialogResult to display dialog "Enter model name to pull (e.g., llama3.2, gemma2:2b):" default answer "" buttons {"Cancel", "Pull"} default button "Pull" with title "Pull Model"
            if button returned of dialogResult is "Pull" then
                return text returned of dialogResult
            else
                return ""
            end if
        "#;

        let output = std::process::Command::new("osascript")
            .args(["-e", script])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let model_name = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !model_name.is_empty() {
                    tracing::info!("Pulling model: {}", model_name);
                    let state = self.state.clone();
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async {
                            match state.pull_model(&model_name).await {
                                Ok(()) => {
                                    tracing::info!("Model pull completed: {}", model_name);
                                    // Show success notification
                                    let _ = std::process::Command::new("osascript")
                                        .args(["-e", &format!(
                                            r#"display notification "Model {} pulled successfully" with title "OllamaBar""#,
                                            model_name
                                        )])
                                        .spawn();
                                }
                                Err(e) => {
                                    tracing::error!("Failed to pull model: {}", e);
                                    // Show error notification
                                    let _ = std::process::Command::new("osascript")
                                        .args(["-e", &format!(
                                            r#"display notification "Failed to pull {}: {}" with title "OllamaBar""#,
                                            model_name, e
                                        )])
                                        .spawn();
                                }
                            }
                        });
                    });
                }
            }
            Ok(_) => {
                // User cancelled or dialog failed
                tracing::debug!("Pull model dialog cancelled");
            }
            Err(e) => {
                tracing::error!("Failed to show pull model dialog: {}", e);
            }
        }
    }

    fn handle_view_logs(&self) {
        use crate::state::AppState;

        let log_path = AppState::ollama_log_path();
        tracing::info!("Opening logs at: {:?}", log_path);

        if log_path.exists() {
            // Open with Console app for nice log viewing
            let _ = std::process::Command::new("open")
                .args(["-a", "Console"])
                .arg(&log_path)
                .spawn();
        } else {
            // Log file doesn't exist yet - show a message
            tracing::warn!("Log file doesn't exist yet: {:?}", log_path);
            // Open Console app anyway so user can see it when logs appear
            let _ = std::process::Command::new("open")
                .args(["-a", "Console"])
                .spawn();
        }
    }

    fn handle_settings(&self) {
        tracing::info!("Opening settings...");
        if let Ok(config_path) = llm_core::Config::find_config_path() {
            let _ = std::process::Command::new("open").arg(config_path).spawn();
        }
    }
}
