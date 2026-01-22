//! Menu bar UI controller

use crate::app::get_action_delegate;
use crate::state::AppState;
use anyhow::Result;
use llm_core::{OllamaStatus, TailscaleStatus};
use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{msg_send, msg_send_id, sel};
use objc2_app_kit::{
    NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{MainThreadMarker, NSString};
use std::time::Duration;

/// Controls the menu bar icon and dropdown menu
pub struct MenuBarController {
    status_item: Retained<NSStatusItem>,
    menu: Retained<NSMenu>,
    state: AppState,
    mtm: MainThreadMarker,
}

impl MenuBarController {
    pub fn new(mtm: MainThreadMarker, state: AppState) -> Result<Self> {
        // Create status bar item
        let status_bar = unsafe { NSStatusBar::systemStatusBar() };
        let status_item = unsafe { status_bar.statusItemWithLength(NSVariableStatusItemLength) };

        // Set initial icon
        if let Some(button) = unsafe { status_item.button(mtm) } {
            let title = NSString::from_str("○"); // Circle icon
            unsafe { button.setTitle(&title) };
        }

        // Create menu
        let menu = NSMenu::new(mtm);

        // Set the menu's delegate to auto-update before showing
        // This ensures fresh state every time the menu opens
        unsafe { status_item.setMenu(Some(&menu)) };

        let mut controller = Self {
            status_item,
            menu,
            state,
            mtm,
        };

        // Build initial menu
        controller.rebuild_menu();

        Ok(controller)
    }

    /// Start background monitoring of Ollama status
    pub fn start_monitoring(&self) {
        let state = self.state.clone();

        // Spawn a thread for async monitoring
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();

            loop {
                rt.block_on(async {
                    if let Err(e) = state.refresh().await {
                        tracing::warn!("Failed to refresh state: {}", e);
                    }
                });

                std::thread::sleep(Duration::from_secs(5));
            }
        });
    }

    /// Rebuild the menu based on current state
    pub fn rebuild_menu(&mut self) {
        unsafe { self.menu.removeAllItems() };

        // Status section
        self.add_status_section();
        self.add_separator();

        // Actions
        self.add_action_section();
        self.add_separator();

        // Model switching
        self.add_model_section();
        self.add_separator();

        // Tailscale
        self.add_tailscale_section();
        self.add_separator();

        // Footer
        self.add_footer_section();
    }

    fn add_status_section(&self) {
        let status = self.state.ollama_status();
        let status_text = match status {
            OllamaStatus::Running => "● Ollama Running",
            OllamaStatus::Starting => "◐ Ollama Starting...",
            OllamaStatus::Stopped => "○ Ollama Stopped",
            OllamaStatus::Error => "⊘ Ollama Error",
        };

        let item = self.create_menu_item(status_text, None);
        unsafe { item.setEnabled(false) };
        self.menu.addItem(&item);

        // Current model
        if let Some(model) = self.state.current_model() {
            let model_text = format!("  Model: {}", model);
            let item = self.create_menu_item(&model_text, None);
            unsafe { item.setEnabled(false) };
            self.menu.addItem(&item);
        }

        // Memory
        let (used, total) = self.state.memory_info();
        let mem_text = format!("  Memory: {:.1} / {:.0} GB", used, total);
        let item = self.create_menu_item(&mem_text, None);
        unsafe { item.setEnabled(false) };
        self.menu.addItem(&item);
    }

    fn add_action_section(&self) {
        let status = self.state.ollama_status();

        if status == OllamaStatus::Running {
            let item = self.create_action_item("Stop Ollama", sel!(stopOllama:));
            self.menu.addItem(&item);
        } else {
            let item = self.create_action_item("Start Ollama", sel!(startOllama:));
            self.menu.addItem(&item);
        }

        let item = self.create_action_item("Restart", sel!(restartOllama:));
        unsafe { item.setEnabled(status == OllamaStatus::Running) };
        self.menu.addItem(&item);
    }

    fn add_model_section(&self) {
        let models = self.state.available_models();
        let current = self.state.current_model();

        if models.is_empty() {
            let item = self.create_menu_item("No models available", None);
            unsafe { item.setEnabled(false) };
            self.menu.addItem(&item);
            return;
        }

        // Create submenu
        let submenu = NSMenu::new(self.mtm);
        let submenu_title = NSString::from_str("Switch Model");

        for model in &models {
            let item = self.create_action_item(model, sel!(switchModel:));

            // Check mark for current model
            if current.as_ref() == Some(model) {
                // NSControlStateValueOn = 1
                unsafe {
                    let _: () = msg_send![&item, setState: 1i64];
                };
            }

            submenu.addItem(&item);
        }

        let item = NSMenuItem::new(self.mtm);
        unsafe { item.setTitle(&submenu_title) };
        item.setSubmenu(Some(&submenu));
        self.menu.addItem(&item);
    }

    fn add_tailscale_section(&self) {
        let ts_status = self.state.tailscale_status();
        let sharing = self.state.tailscale_sharing();

        let status_text = match ts_status {
            TailscaleStatus::Connected => "Tailscale: Connected",
            TailscaleStatus::Disconnected => "Tailscale: Disconnected",
            TailscaleStatus::NotInstalled => "Tailscale: Not Installed",
        };

        let item = self.create_menu_item(status_text, None);
        unsafe { item.setEnabled(false) };
        self.menu.addItem(&item);

        // Share toggle
        if ts_status == TailscaleStatus::Connected {
            let share_text = if sharing {
                "☑ Share via Tailscale"
            } else {
                "☐ Share via Tailscale"
            };

            let item = self.create_action_item(share_text, sel!(toggleTailscale:));
            self.menu.addItem(&item);

            // Show URL if sharing
            if sharing {
                if let Some(ip) = self.state.tailscale_ip() {
                    let url = format!("  http://{}:11434  [Click to copy]", ip);
                    let item = self.create_action_item(&url, sel!(copyTailscaleUrl:));
                    self.menu.addItem(&item);
                }
            }
        }
    }

    fn add_footer_section(&self) {
        let item = self.create_action_item("Pull Model...", sel!(pullModel:));
        self.menu.addItem(&item);

        let item = self.create_action_item("View Logs", sel!(viewLogs:));
        self.menu.addItem(&item);

        let item = self.create_action_item("Settings...", sel!(openSettings:));
        self.menu.addItem(&item);

        self.add_separator();

        // Version info
        let version = env!("CARGO_PKG_VERSION");
        let version_text = format!("OllamaBar v{}", version);
        let item = self.create_menu_item(&version_text, None);
        unsafe { item.setEnabled(false) };
        self.menu.addItem(&item);

        // Quit uses the app's terminate: action
        let item = self.create_menu_item("Quit", Some(sel!(terminate:)));
        self.menu.addItem(&item);
    }

    fn add_separator(&self) {
        let sep = NSMenuItem::separatorItem(self.mtm);
        self.menu.addItem(&sep);
    }

    /// Update the menu bar icon based on status
    pub fn update_icon(&self) {
        let status = self.state.ollama_status();
        let sharing = self.state.tailscale_sharing();

        let icon = match (status, sharing) {
            (OllamaStatus::Running, true) => "●↗",
            (OllamaStatus::Running, false) => "●",
            (OllamaStatus::Starting, _) => "◐",
            (OllamaStatus::Stopped, _) => "○",
            (OllamaStatus::Error, _) => "⊘",
        };

        if let Some(button) = unsafe { self.status_item.button(self.mtm) } {
            let title = NSString::from_str(icon);
            unsafe { button.setTitle(&title) };
        }
    }

    /// Create a menu item without action
    fn create_menu_item(&self, title: &str, action: Option<Sel>) -> Retained<NSMenuItem> {
        let title_ns = NSString::from_str(title);
        let key = NSString::from_str("");

        unsafe {
            let item: Retained<NSMenuItem> = msg_send_id![
                self.mtm.alloc::<NSMenuItem>(),
                initWithTitle: &*title_ns,
                action: action,
                keyEquivalent: &*key
            ];
            item
        }
    }

    /// Create a menu item with action targeted at our ActionDelegate
    fn create_action_item(&self, title: &str, action: Sel) -> Retained<NSMenuItem> {
        let item = self.create_menu_item(title, Some(action));

        // Set the target to our action delegate
        if let Some(delegate) = get_action_delegate() {
            unsafe {
                let _: () = msg_send![&item, setTarget: &*delegate];
            }
        }

        item
    }
}
