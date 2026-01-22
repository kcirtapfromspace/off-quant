//! Action handlers for menu items

use crate::app::get_mtm;
use crate::dialogs::{pull_model_with_progress, show_pull_model_dialog};
use crate::notifications::Notification;
use crate::state::AppState;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{declare_class, msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, NSString};
use std::cell::RefCell;
use std::process::Command;

thread_local! {
    static ACTION_STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

/// Set the app state for action handlers
pub fn set_action_state(state: AppState) {
    ACTION_STATE.with(|s| {
        *s.borrow_mut() = Some(state);
    });
}

fn get_state() -> Option<AppState> {
    ACTION_STATE.with(|s| s.borrow().clone())
}

// Declare the ActionDelegate class that handles menu actions
declare_class!(
    pub struct ActionDelegate;

    unsafe impl ClassType for ActionDelegate {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "OllamaBarActionDelegate";
    }

    impl DeclaredClass for ActionDelegate {
        type Ivars = ();
    }

    unsafe impl NSObjectProtocol for ActionDelegate {}

    // Action handlers
    unsafe impl ActionDelegate {
        #[method(startOllama:)]
        fn start_ollama(&self, _sender: *mut AnyObject) {
            tracing::info!("Action: Start Ollama");

            if let Some(state) = get_state() {
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    rt.block_on(async move {
                        match state.start_ollama().await {
                            Ok(()) => {
                                tracing::info!("Ollama started successfully");
                                Notification::OllamaStarted.send();
                            }
                            Err(e) => {
                                tracing::error!("Failed to start Ollama: {}", e);
                                Notification::OllamaError(e.to_string()).send();
                            }
                        }
                    });
                });
            }
        }

        #[method(stopOllama:)]
        fn stop_ollama(&self, _sender: *mut AnyObject) {
            tracing::info!("Action: Stop Ollama");

            if let Some(state) = get_state() {
                match state.stop_ollama() {
                    Ok(()) => {
                        tracing::info!("Ollama stopped");
                        Notification::OllamaStopped.send();
                    }
                    Err(e) => {
                        tracing::error!("Failed to stop Ollama: {}", e);
                        Notification::OllamaError(e.to_string()).send();
                    }
                }
            }
        }

        #[method(restartOllama:)]
        fn restart_ollama(&self, _sender: *mut AnyObject) {
            tracing::info!("Action: Restart Ollama");

            if let Some(state) = get_state() {
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    rt.block_on(async move {
                        // Stop first
                        if let Err(e) = state.stop_ollama() {
                            tracing::error!("Failed to stop Ollama: {}", e);
                            Notification::OllamaError(e.to_string()).send();
                            return;
                        }

                        // Wait a moment
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

                        // Start again
                        match state.start_ollama().await {
                            Ok(()) => {
                                tracing::info!("Ollama restarted successfully");
                                Notification::OllamaStarted.send();
                            }
                            Err(e) => {
                                tracing::error!("Failed to restart Ollama: {}", e);
                                Notification::OllamaError(e.to_string()).send();
                            }
                        }
                    });
                });
            }
        }

        #[method(switchModel:)]
        fn switch_model(&self, sender: *mut AnyObject) {
            // Get the model name from the menu item title
            let sender: &AnyObject = unsafe { &*sender };

            let title: Option<Retained<NSString>> = unsafe {
                msg_send_id![sender, title]
            };

            if let Some(title) = title {
                let model = title.to_string();
                tracing::info!("Action: Switch to model {}", model);

                if let Some(state) = get_state() {
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async move {
                            match state.switch_model(&model).await {
                                Ok(()) => {
                                    tracing::info!("Switched to model: {}", model);
                                    Notification::ModelLoaded(model).send();
                                }
                                Err(e) => {
                                    tracing::error!("Failed to switch model: {}", e);
                                    Notification::OllamaError(e.to_string()).send();
                                }
                            }
                        });
                    });
                }
            }
        }

        #[method(toggleTailscale:)]
        fn toggle_tailscale(&self, _sender: *mut AnyObject) {
            tracing::info!("Action: Toggle Tailscale sharing");

            if let Some(state) = get_state() {
                let was_sharing = state.tailscale_sharing();

                match state.toggle_tailscale_sharing() {
                    Ok(()) => {
                        if was_sharing {
                            Notification::TailscaleDisabled.send();
                        } else if let Some(ip) = state.tailscale_ip() {
                            let url = format!("http://{}:11434", ip);
                            Notification::TailscaleEnabled(url).send();
                        }
                        tracing::info!("Tailscale sharing toggled");
                    }
                    Err(e) => {
                        tracing::error!("Failed to toggle Tailscale: {}", e);
                        Notification::OllamaError(e.to_string()).send();
                    }
                }
            }
        }

        #[method(copyTailscaleUrl:)]
        fn copy_tailscale_url(&self, _sender: *mut AnyObject) {
            tracing::info!("Action: Copy Tailscale URL");

            if let Some(state) = get_state() {
                if let Some(ip) = state.tailscale_ip() {
                    let url = format!("http://{}:11434", ip);

                    // Copy to clipboard using pbcopy
                    if let Ok(mut child) = Command::new("pbcopy")
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                    {
                        use std::io::Write;
                        if let Some(stdin) = child.stdin.as_mut() {
                            let _ = stdin.write_all(url.as_bytes());
                        }
                        let _ = child.wait();
                        tracing::info!("Copied to clipboard: {}", url);
                        Notification::UrlCopied.send();
                    }
                }
            }
        }

        #[method(pullModel:)]
        fn pull_model(&self, _sender: *mut AnyObject) {
            tracing::info!("Action: Pull model");

            // Show native dialog on main thread
            if let Some(mtm) = get_mtm() {
                if let Some(model_name) = show_pull_model_dialog(mtm) {
                    tracing::info!("User requested to pull: {}", model_name);

                    if let Some(state) = get_state() {
                        // Start download in background
                        pull_model_with_progress(state, model_name);
                    }
                }
            }
        }

        #[method(viewLogs:)]
        fn view_logs(&self, _sender: *mut AnyObject) {
            tracing::info!("Action: View logs");

            // Open Console.app with Ollama logs
            let _ = Command::new("open")
                .args(["-a", "Console", "/tmp/ollama.out.log"])
                .spawn();
        }

        #[method(openSettings:)]
        fn open_settings(&self, _sender: *mut AnyObject) {
            tracing::info!("Action: Open settings");

            // Open llm.toml in default editor
            if let Ok(config_path) = llm_core::Config::find_config_path() {
                let _ = Command::new("open")
                    .arg(config_path)
                    .spawn();
            }
        }
    }
);

impl ActionDelegate {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(());
        unsafe { msg_send_id![super(this), init] }
    }
}
