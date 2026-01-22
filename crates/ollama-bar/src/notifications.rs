//! System notifications for OllamaBar

use std::process::Command;

/// Send a macOS notification
pub fn send_notification(title: &str, message: &str) {
    // Use osascript for notifications (doesn't require entitlements)
    let script = format!(
        r#"display notification "{}" with title "{}""#,
        message.replace('"', "\\\""),
        title.replace('"', "\\\"")
    );

    let _ = Command::new("osascript").args(["-e", &script]).spawn();

    tracing::debug!("Notification: {} - {}", title, message);
}

/// Notification types for the app
pub enum Notification {
    OllamaStarted,
    OllamaStopped,
    OllamaError(String),
    ModelLoaded(String),
    ModelDownloadComplete(String),
    ModelDownloadFailed(String),
    TailscaleEnabled(String),
    TailscaleDisabled,
    UrlCopied,
}

impl Notification {
    pub fn send(&self) {
        let (title, message) = match self {
            Notification::OllamaStarted => ("OllamaBar", "Ollama is now running"),
            Notification::OllamaStopped => ("OllamaBar", "Ollama has stopped"),
            Notification::OllamaError(e) => ("OllamaBar Error", e.as_str()),
            Notification::ModelLoaded(name) => {
                return send_notification("Model Loaded", &format!("{} is ready", name));
            }
            Notification::ModelDownloadComplete(name) => {
                return send_notification(
                    "Download Complete",
                    &format!("{} is ready to use", name),
                );
            }
            Notification::ModelDownloadFailed(name) => {
                return send_notification(
                    "Download Failed",
                    &format!("Failed to download {}", name),
                );
            }
            Notification::TailscaleEnabled(url) => {
                return send_notification("Tailscale Sharing", &format!("Available at {}", url));
            }
            Notification::TailscaleDisabled => ("OllamaBar", "Tailscale sharing disabled"),
            Notification::UrlCopied => ("OllamaBar", "URL copied to clipboard"),
        };

        send_notification(title, message);
    }
}
