//! OllamaBar - macOS menu bar app for local LLM management
//!
//! A native menu bar application that provides:
//! - One-click Ollama start/stop
//! - Model switching
//! - Tailscale network sharing
//! - Memory monitoring

#![cfg(target_os = "macos")]

mod state;
mod tray;

use anyhow::Result;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env()
            .add_directive("ollama_bar=debug".parse()?)
            .add_directive("llm_core=debug".parse()?))
        .init();

    tracing::info!("Starting OllamaBar");

    run_with_tray()
}

fn run_with_tray() -> Result<()> {
    use crate::state::AppState;
    use crate::tray::TrayManager;
    use muda::MenuEvent;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2::{declare_class, msg_send_id, mutability, sel, ClassType, DeclaredClass};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate};
    use objc2_foundation::{
        MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSTimer,
    };
    use std::cell::RefCell;
    use std::time::Duration;

    // Thread-local storage for tray manager (main thread only)
    thread_local! {
        static TRAY_MANAGER: RefCell<Option<TrayManager>> = const { RefCell::new(None) };
    }

    // Shared state for background refresh
    let state = AppState::new()?;
    let state_for_monitor = state.clone();

    // Minimal app delegate
    declare_class!(
        struct TrayAppDelegate;

        unsafe impl ClassType for TrayAppDelegate {
            type Super = NSObject;
            type Mutability = mutability::MainThreadOnly;
            const NAME: &'static str = "OllamaBarTrayAppDelegate";
        }

        impl DeclaredClass for TrayAppDelegate {
            type Ivars = ();
        }

        unsafe impl NSObjectProtocol for TrayAppDelegate {}

        unsafe impl NSApplicationDelegate for TrayAppDelegate {
            #[method(applicationDidFinishLaunching:)]
            fn did_finish_launching(&self, _notification: &NSNotification) {
                tracing::info!("Application did finish launching");
            }

            #[method(applicationWillTerminate:)]
            fn will_terminate(&self, _notification: &NSNotification) {
                tracing::info!("Application will terminate");
            }
        }
    );

    impl TrayAppDelegate {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = mtm.alloc::<Self>();
            let this = this.set_ivars(());
            unsafe { msg_send_id![super(this), init] }
        }
    }

    // Timer delegate for processing events on main thread
    declare_class!(
        struct TrayTimerDelegate;

        unsafe impl ClassType for TrayTimerDelegate {
            type Super = NSObject;
            type Mutability = mutability::MainThreadOnly;
            const NAME: &'static str = "OllamaBarTrayTimerDelegate";
        }

        impl DeclaredClass for TrayTimerDelegate {
            type Ivars = ();
        }

        unsafe impl NSObjectProtocol for TrayTimerDelegate {}

        unsafe impl TrayTimerDelegate {
            #[method(timerFired:)]
            fn timer_fired(&self, _timer: *mut AnyObject) {
                // Process menu events on main thread
                let menu_rx = MenuEvent::receiver();

                while let Ok(event) = menu_rx.try_recv() {
                    tracing::info!("Menu event: {:?}", event.id);

                    TRAY_MANAGER.with(|tm| {
                        if let Some(manager) = tm.borrow().as_ref() {
                            let quit = manager.handle_event(&event);
                            if quit {
                                unsafe {
                                    if let Some(mtm) = MainThreadMarker::new() {
                                        let app = NSApplication::sharedApplication(mtm);
                                        app.terminate(None);
                                    }
                                }
                            }
                        }
                    });
                }

                // Update menu/icon
                TRAY_MANAGER.with(|tm| {
                    if let Some(manager) = tm.borrow_mut().as_mut() {
                        let _ = manager.update_menu();
                        manager.update_icon();
                    }
                });
            }
        }
    );

    impl TrayTimerDelegate {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = mtm.alloc::<Self>();
            let this = this.set_ivars(());
            unsafe { msg_send_id![super(this), init] }
        }
    }

    // Main thread setup
    let mtm = MainThreadMarker::new().expect("Must run on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    // Create tray manager and store in thread-local
    let manager = TrayManager::new(state)?;
    TRAY_MANAGER.with(|tm| {
        *tm.borrow_mut() = Some(manager);
    });

    // Create tray icon
    TRAY_MANAGER.with(|tm| {
        if let Some(manager) = tm.borrow_mut().as_mut() {
            if let Err(e) = manager.create_tray() {
                tracing::error!("Failed to create tray: {}", e);
            }
        }
    });

    // Set up app delegate
    let delegate = TrayAppDelegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    // Set up timer for event processing (runs on main thread)
    let timer_delegate = TrayTimerDelegate::new(mtm);
    unsafe {
        let _timer: Retained<NSTimer> = msg_send_id![
            NSTimer::class(),
            scheduledTimerWithTimeInterval: 1.0f64,
            target: &*timer_delegate,
            selector: sel!(timerFired:),
            userInfo: std::ptr::null::<AnyObject>(),
            repeats: true
        ];
    }

    // Keep timer delegate alive
    thread_local! {
        static TIMER_DELEGATE: RefCell<Option<Retained<TrayTimerDelegate>>> = const { RefCell::new(None) };
    }
    TIMER_DELEGATE.with(|td| *td.borrow_mut() = Some(timer_delegate));

    tracing::info!("OllamaBar running - check your menu bar");

    // Start background monitoring thread (only refreshes AppState, doesn't touch TrayManager)
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        loop {
            rt.block_on(async {
                let _ = state_for_monitor.refresh().await;
            });
            std::thread::sleep(Duration::from_secs(5));
        }
    });

    // Run the app (blocks)
    unsafe { app.run() };

    Ok(())
}
