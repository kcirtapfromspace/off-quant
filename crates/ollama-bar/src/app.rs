//! macOS application setup using AppKit via objc2

use crate::actions::{set_action_state, ActionDelegate};
use crate::menu::MenuBarController;
use crate::state::AppState;
use anyhow::Result;
use llm_core::OllamaStatus;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{declare_class, msg_send_id, mutability, sel, ClassType, DeclaredClass};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSRunLoop, NSTimer,
};
use std::cell::RefCell;

thread_local! {
    static MENU_CONTROLLER: RefCell<Option<MenuBarController>> = const { RefCell::new(None) };
    static ACTION_DELEGATE: RefCell<Option<Retained<ActionDelegate>>> = const { RefCell::new(None) };
    static APP_STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
    static LAST_STATUS: RefCell<OllamaStatus> = const { RefCell::new(OllamaStatus::Stopped) };
}

/// Get the action delegate for setting menu item targets
pub fn get_action_delegate() -> Option<Retained<ActionDelegate>> {
    ACTION_DELEGATE.with(|ad| ad.borrow().clone())
}

/// Get the main thread marker (only valid on main thread)
pub fn get_mtm() -> Option<MainThreadMarker> {
    MainThreadMarker::new()
}

/// Request a menu rebuild (called from timer)
fn refresh_ui() {
    // Get current status
    let new_status = APP_STATE.with(|s| {
        s.borrow().as_ref().map(|state| state.ollama_status())
    });

    let old_status = LAST_STATUS.with(|s| *s.borrow());

    // Only rebuild if status changed
    if let Some(new_status) = new_status {
        if new_status != old_status {
            LAST_STATUS.with(|s| *s.borrow_mut() = new_status);

            // Rebuild menu
            MENU_CONTROLLER.with(|mc| {
                if let Some(controller) = mc.borrow_mut().as_mut() {
                    controller.rebuild_menu();
                    controller.update_icon();
                }
            });

            tracing::debug!("UI refreshed: {:?} -> {:?}", old_status, new_status);
        }
    }
}

/// Run the menu bar application
pub fn run() -> Result<()> {
    // Ensure we're on the main thread for AppKit
    let mtm = MainThreadMarker::new().expect("Must run on main thread");

    // Get shared application
    let app = NSApplication::sharedApplication(mtm);

    // Set activation policy to accessory (menu bar only, no dock icon)
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    // Initialize our state
    let state = AppState::new()?;

    // Store state for action handlers and UI refresh
    set_action_state(state.clone());
    APP_STATE.with(|s| *s.borrow_mut() = Some(state.clone()));

    // Create action delegate
    let action_delegate = ActionDelegate::new(mtm);
    ACTION_DELEGATE.with(|ad| {
        *ad.borrow_mut() = Some(action_delegate);
    });

    // Create menu bar controller
    let menu_controller = MenuBarController::new(mtm, state)?;
    MENU_CONTROLLER.with(|mc| {
        *mc.borrow_mut() = Some(menu_controller);
    });

    // Create and set app delegate
    let delegate = AppDelegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    tracing::info!("OllamaBar running - check your menu bar");

    // Run the app (blocks forever)
    unsafe { app.run() };

    Ok(())
}

// Application delegate to handle app lifecycle
declare_class!(
    struct AppDelegate;

    unsafe impl ClassType for AppDelegate {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "OllamaBarAppDelegate";
    }

    impl DeclaredClass for AppDelegate {
        type Ivars = ();
    }

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[method(applicationDidFinishLaunching:)]
        fn application_did_finish_launching(&self, _notification: &NSNotification) {
            tracing::info!("Application did finish launching");

            // Start background status monitoring
            MENU_CONTROLLER.with(|mc| {
                if let Some(controller) = mc.borrow().as_ref() {
                    controller.start_monitoring();
                }
            });

            // Schedule UI refresh timer on main run loop
            let mtm = MainThreadMarker::new().unwrap();
            schedule_ui_timer(mtm);
        }

        #[method(applicationWillTerminate:)]
        fn application_will_terminate(&self, _notification: &NSNotification) {
            tracing::info!("Application will terminate");
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(());
        unsafe { msg_send_id![super(this), init] }
    }
}

// Timer callback delegate
declare_class!(
    struct TimerDelegate;

    unsafe impl ClassType for TimerDelegate {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "OllamaBarTimerDelegate";
    }

    impl DeclaredClass for TimerDelegate {
        type Ivars = ();
    }

    unsafe impl NSObjectProtocol for TimerDelegate {}

    unsafe impl TimerDelegate {
        #[method(timerFired:)]
        fn timer_fired(&self, _timer: *mut AnyObject) {
            refresh_ui();
        }
    }
);

impl TimerDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(());
        unsafe { msg_send_id![super(this), init] }
    }
}

/// Schedule a repeating timer for UI updates
fn schedule_ui_timer(mtm: MainThreadMarker) {
    let delegate = TimerDelegate::new(mtm);

    // Create a repeating timer (every 2 seconds)
    unsafe {
        let _timer: Retained<NSTimer> = msg_send_id![
            NSTimer::class(),
            scheduledTimerWithTimeInterval: 2.0f64,
            target: &*delegate,
            selector: sel!(timerFired:),
            userInfo: std::ptr::null::<AnyObject>(),
            repeats: true
        ];

        // Keep delegate alive by adding to run loop's context
        // The timer retains its target, so delegate stays alive
        let _run_loop = NSRunLoop::currentRunLoop();

        // Store delegate in thread local to prevent deallocation
        thread_local! {
            static TIMER_DELEGATE: RefCell<Option<Retained<TimerDelegate>>> = const { RefCell::new(None) };
        }
        TIMER_DELEGATE.with(|td| *td.borrow_mut() = Some(delegate));

        tracing::debug!("UI refresh timer scheduled");
    }
}
