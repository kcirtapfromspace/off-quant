//! Native dialogs for OllamaBar

use crate::notifications::Notification;
use crate::state::AppState;
use objc2::msg_send_id;
use objc2::rc::Retained;
use objc2_app_kit::{NSAlert, NSAlertStyle, NSTextField};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

/// Show a dialog to pull a new model
pub fn show_pull_model_dialog(mtm: MainThreadMarker) -> Option<String> {
    unsafe {
        // Create alert
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("Pull Model"));
        alert.setInformativeText(&NSString::from_str(
            "Enter the model name to download from Ollama registry.\nExamples: llama3.2, qwen2.5-coder:7b, deepseek-coder:6.7b"
        ));
        alert.setAlertStyle(NSAlertStyle::Informational);

        // Add buttons
        alert.addButtonWithTitle(&NSString::from_str("Pull"));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));

        // Create text field for input
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(300.0, 24.0));
        let text_field: Retained<NSTextField> = msg_send_id![
            mtm.alloc::<NSTextField>(),
            initWithFrame: frame
        ];
        text_field.setPlaceholderString(Some(&NSString::from_str("model-name:tag")));

        // Set as accessory view
        alert.setAccessoryView(Some(&text_field));

        // Make text field first responder
        let window = alert.window();
        window.makeFirstResponder(Some(&text_field));

        // Run modal
        let response = alert.runModal();

        // NSAlertFirstButtonReturn = 1000
        if response == 1000 {
            let value = text_field.stringValue();
            let model_name = value.to_string();
            if !model_name.is_empty() {
                return Some(model_name);
            }
        }

        None
    }
}

/// Show an error alert
#[allow(dead_code)]
pub fn show_error_alert(mtm: MainThreadMarker, title: &str, message: &str) {
    unsafe {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(message));
        alert.setAlertStyle(NSAlertStyle::Critical);
        alert.addButtonWithTitle(&NSString::from_str("OK"));
        alert.runModal();
    }
}

/// Show a confirmation dialog
#[allow(dead_code)]
pub fn show_confirmation(mtm: MainThreadMarker, title: &str, message: &str) -> bool {
    unsafe {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(message));
        alert.setAlertStyle(NSAlertStyle::Warning);
        alert.addButtonWithTitle(&NSString::from_str("Yes"));
        alert.addButtonWithTitle(&NSString::from_str("No"));

        let response = alert.runModal();
        response == 1000 // NSAlertFirstButtonReturn
    }
}

/// Pull a model with progress feedback
pub fn pull_model_with_progress(state: AppState, model_name: String) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            tracing::info!("Starting download of model: {}", model_name);

            // Use ollama pull command
            let output = tokio::process::Command::new("ollama")
                .args(["pull", &model_name])
                .output()
                .await;

            match output {
                Ok(out) if out.status.success() => {
                    tracing::info!("Model {} downloaded successfully", model_name);
                    Notification::ModelDownloadComplete(model_name).send();

                    // Refresh model list
                    let _ = state.refresh().await;
                }
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    tracing::error!("Failed to download {}: {}", model_name, stderr);
                    Notification::ModelDownloadFailed(model_name).send();
                }
                Err(e) => {
                    tracing::error!("Failed to run ollama pull: {}", e);
                    Notification::ModelDownloadFailed(model_name).send();
                }
            }
        });
    });
}
