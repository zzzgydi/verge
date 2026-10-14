//! Isolated native tray regression probe. Never starts a backend or writes system settings.
//! Run from a logged-in macOS desktop: cargo run -p verge --example tray_proxy_probe
pub use verge::{domain, identity};
#[path = "../src/i18n.rs"]
#[allow(dead_code)]
mod i18n;

#[allow(dead_code)]
mod native {
    include!("../src/platform/tray.rs");

    pub fn verify() {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{
            NSApplication, NSApplicationActivationPolicy, NSEventTrackingRunLoopMode,
        };
        use objc2_foundation::{NSDate, NSRunLoop};
        use std::{
            sync::mpsc,
            time::{Duration, Instant},
        };
        use tray_icon::menu::ContextMenu;

        let marker = MainThreadMarker::new().unwrap();
        let app = NSApplication::sharedApplication(marker);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        assert!(
            !crate::identity::AppChannel::current().is_dev(),
            "run without VERGE_BUILD_CHANNEL=dev"
        );
        let (tx, rx) = mpsc::channel();
        let bound = TrayService::new()
            .unwrap()
            .bind(move |command| tx.send(command).unwrap());
        let tray = bound.get(marker);
        let mut snapshot = TraySnapshot {
            menu: Arc::new(TrayMenuState {
                language: "en".into(),
                can_enable_system_proxy: true,
                mode: Some(RunMode::Rule),
                groups: vec![ProxyGroup {
                    name: "Isolated fixture".into(),
                    kind: "Selector".into(),
                    selected: Some("Node 0".into()),
                    members: (0..300).map(|i| format!("Node {i}")).collect(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        tray.update(&snapshot).unwrap();
        let status = tray.tray.ns_status_item().unwrap();
        let menu = unsafe {
            objc2::rc::Retained::retain(
                tray.menu.borrow().ns_menu().cast::<objc2_app_kit::NSMenu>(),
            )
            .unwrap()
        };
        let proxy = menu.itemAtIndex(7).unwrap();
        let nodes = menu.itemAtIndex(5).unwrap();
        let original_image = status
            .button(marker)
            .unwrap()
            .image()
            .unwrap()
            .TIFFRepresentation()
            .unwrap()
            .to_vec();

        let started = Instant::now();
        menu.performActionForItemAtIndex(7);
        let feedback = started.elapsed();
        assert_eq!(
            rx.try_recv().unwrap(),
            TrayCommand::SetSystemProxy {
                enabled: true,
                request_id: 1
            }
        );
        assert!(!proxy.isEnabled());
        assert_eq!(proxy.state(), 0);
        assert!(proxy.title().to_string().contains("Switching..."));
        assert!(
            tray.menu_command(tray.proxy_item.borrow().as_ref().unwrap().id().as_ref())
                .is_none()
        );
        assert!(rx.try_recv().is_err());

        // Stale traffic must neither clear pending nor show successful proxy state.
        tray.update(&snapshot).unwrap();
        assert!(!proxy.isEnabled());
        assert_eq!(proxy.state(), 0);
        assert_eq!(
            status
                .button(marker)
                .unwrap()
                .image()
                .unwrap()
                .TIFFRepresentation()
                .unwrap()
                .to_vec(),
            original_image
        );

        // Failure acknowledges the request while retaining confirmed state and native items.
        snapshot.completed_system_proxy_request = 1;
        tray.update(&snapshot).unwrap();
        assert!(proxy.isEnabled());
        assert_eq!(proxy.state(), 0);
        assert!(!proxy.title().to_string().contains("Switching..."));
        menu.performActionForItemAtIndex(7);
        assert_eq!(
            rx.try_recv().unwrap(),
            TrayCommand::SetSystemProxy {
                enabled: true,
                request_id: 2
            }
        );
        tray.update(&snapshot).unwrap();
        assert!(
            !proxy.isEnabled(),
            "old completion cannot clear second request"
        );

        let updates = TrayUpdateSender::new(bound.clone());
        let mut success = snapshot.clone();
        Arc::make_mut(&mut success.menu).system_proxy_enabled = true;
        success.indicator = TrayIndicator::SystemProxy;
        success.completed_system_proxy_request = 2;
        let sender = updates.clone();
        let started = Instant::now();
        std::thread::spawn(move || {
            sender.send(snapshot);
            sender.send(success);
        })
        .join()
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !proxy.isEnabled() && Instant::now() < deadline {
            // Exercise the same nested mode used while the status menu tracks the mouse.
            unsafe {
                NSRunLoop::mainRunLoop().runMode_beforeDate(
                    NSEventTrackingRunLoopMode,
                    &NSDate::dateWithTimeIntervalSinceNow(0.01),
                );
            }
        }
        assert!(
            proxy.isEnabled(),
            "completion did not run in menu-tracking mode"
        );
        let delivered = started.elapsed();
        assert_eq!(proxy.state(), 1);
        assert_eq!(tray.indicator.get(), TrayIndicator::SystemProxy);
        assert_ne!(
            status
                .button(marker)
                .unwrap()
                .image()
                .unwrap()
                .TIFFRepresentation()
                .unwrap()
                .to_vec(),
            original_image
        );
        assert!(std::ptr::eq(
            tray.menu.borrow().ns_menu().cast::<objc2_app_kit::NSMenu>(),
            &*menu
        ));
        assert!(std::ptr::eq(&*menu.itemAtIndex(7).unwrap(), &*proxy));
        assert!(std::ptr::eq(&*menu.itemAtIndex(5).unwrap(), &*nodes));
        menu.performActionForItemAtIndex(7);
        assert_eq!(
            rx.try_recv().unwrap(),
            TrayCommand::SetSystemProxy {
                enabled: false,
                request_id: 3
            }
        );

        // Disable completion also uses normal run-loop mode, retaining TUN precedence.
        let mut disabled = TraySnapshot {
            menu: tray.state.borrow().as_ref().unwrap().clone(),
            indicator: TrayIndicator::Tun,
            completed_system_proxy_request: 3,
            ..Default::default()
        };
        Arc::make_mut(&mut disabled.menu).system_proxy_enabled = false;
        std::thread::spawn(move || updates.send(disabled))
            .join()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !proxy.isEnabled() && Instant::now() < deadline {
            unsafe {
                NSRunLoop::mainRunLoop().runMode_beforeDate(
                    objc2_foundation::NSDefaultRunLoopMode,
                    &NSDate::dateWithTimeIntervalSinceNow(0.01),
                );
            }
        }
        assert!(proxy.isEnabled());
        assert_eq!(proxy.state(), 0);
        assert_eq!(tray.indicator.get(), TrayIndicator::Tun);
        println!(
            "PASS: native pending, failure, stale completion, duplicate click, enable/disable intent, latest-only delivery, tracking/default modes, icon change, TUN precedence, stable 300-node menu; feedback_us={}, completion_us={}",
            feedback.as_micros(),
            delivered.as_micros()
        );
    }

    pub fn verify_tracking_refresh() {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSEventTrackingRunLoopMode, NSMenu};
        use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop, NSTimer};
        use std::{
            sync::mpsc,
            time::{Duration, Instant},
        };
        use tray_icon::menu::ContextMenu;

        let marker = MainThreadMarker::new().unwrap();
        let app = objc2_app_kit::NSApplication::sharedApplication(marker);
        app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Accessory);
        let (tx, rx) = mpsc::channel();
        let bound = TrayService::new()
            .unwrap()
            .bind(move |command| tx.send(command).unwrap());
        let tray = bound.get(marker);
        let snapshot = TraySnapshot {
            menu: Arc::new(TrayMenuState {
                language: "en".into(),
                can_enable_system_proxy: true,
                mode: Some(RunMode::Rule),
                groups: vec![ProxyGroup {
                    name: "Tracking fixture".into(),
                    kind: "Selector".into(),
                    selected: Some("A".into()),
                    members: vec!["A".into(), "B".into(), "C".into()],
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        tray.update(&snapshot).unwrap();
        let menu = unsafe {
            objc2::rc::Retained::retain(tray.menu.borrow().ns_menu().cast::<NSMenu>()).unwrap()
        };
        let status = tray.tray.ns_status_item().unwrap();
        let old_id = tray.proxy_item.borrow().as_ref().unwrap().id().clone();
        let observations = Arc::new(Mutex::new(Vec::new()));
        let observed = observations.clone();
        let active = Arc::new(dispatch2::MainThreadBound::new(
            (bound.clone(), menu.clone()),
            marker,
        ));
        let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSTimer>| {
            let marker = MainThreadMarker::new().unwrap();
            let (bound, menu) = active.get(marker);
            let tray = bound.get(marker);
            let mut refreshed = snapshot.clone();
            Arc::make_mut(&mut refreshed.menu).groups[0].selected = Some("B".into());
            tray.update(&refreshed).unwrap();
            let proxy = menu.itemAtIndex(7).unwrap();
            let mut observations = observed.lock().unwrap();
            observations.push((
                "menu survives structural refresh",
                tray.proxy_item.borrow().as_ref().unwrap().id() == &old_id,
            ));
            // Another refresh supersedes the deferred structure, not the visible actions.
            Arc::make_mut(&mut refreshed.menu).groups[0].selected = Some("C".into());
            tray.update(&refreshed).unwrap();
            menu.performActionForItemAtIndex(7);
            observations.push((
                "visible proxy shows pending",
                proxy.title().to_string().contains("Switching..."),
            ));
            // Completion must update the visible item even while structure is deferred.
            refreshed.completed_system_proxy_request = 1;
            refreshed.indicator = TrayIndicator::SystemProxy;
            Arc::make_mut(&mut refreshed.menu).system_proxy_enabled = true;
            tray.update(&refreshed).unwrap();
            observations.push((
                "visible proxy completes",
                proxy.isEnabled() && proxy.state() == 1,
            ));
            observations.push((
                "icon completes during tracking",
                tray.indicator.get() == TrayIndicator::SystemProxy,
            ));
            // Cocoa can deliver the selected item's action after didEndTracking.
            menu.cancelTrackingWithoutAnimation();
        });
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(0.05, false, &block) };
        unsafe {
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSEventTrackingRunLoopMode);
        }
        status.setMenu(Some(&menu));
        unsafe {
            status.button(marker).unwrap().performClick(None);
        }
        status.setMenu(None);
        let proxy_request = rx.try_recv();
        menu.itemAtIndex(2)
            .unwrap()
            .submenu()
            .unwrap()
            .performActionForItemAtIndex(1);
        let mode_request = rx.try_recv();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            unsafe {
                NSRunLoop::mainRunLoop().runMode_beforeDate(
                    NSDefaultRunLoopMode,
                    &NSDate::dateWithTimeIntervalSinceNow(0.01),
                );
            }
            if tray.state.borrow().as_ref().unwrap().groups[0]
                .selected
                .as_deref()
                == Some("C")
            {
                break;
            }
        }
        for (label, ok) in observations.lock().unwrap().iter() {
            assert!(*ok, "{label}");
        }
        assert_eq!(
            proxy_request.unwrap(),
            TrayCommand::SetSystemProxy {
                enabled: true,
                request_id: 1
            }
        );
        assert_eq!(mode_request.unwrap(), TrayCommand::SetMode(RunMode::Global));
        assert_eq!(
            tray.state.borrow().as_ref().unwrap().groups[0]
                .selected
                .as_deref(),
            Some("C")
        );
        assert!(tray.proxy_item.borrow().as_ref().unwrap().is_checked());

        // Unrelated menus cannot put the tray into tracking state.
        let unrelated = NSMenu::new(marker);
        unsafe {
            objc2_foundation::NSNotificationCenter::defaultCenter().postNotificationName_object(
                objc2_app_kit::NSMenuDidBeginTrackingNotification,
                Some(&unrelated),
            );
        }
        assert!(!tray.tracking.borrow().active);

        // If a later refresh restores the visible structure, discard deferred stale work.
        let visible = tray.state.borrow().as_ref().unwrap().clone();
        let current_id = tray.proxy_item.borrow().as_ref().unwrap().id().clone();
        tray.tracking.borrow_mut().begin();
        let mut different = visible.clone();
        Arc::make_mut(&mut different).mode = Some(RunMode::Direct);
        tray.update_menu(&different).unwrap();
        assert!(tray.deferred_menu.borrow().is_some());
        tray.update_menu(&visible).unwrap();
        assert!(tray.deferred_menu.borrow().is_none());
        assert_eq!(tray.proxy_item.borrow().as_ref().unwrap().id(), &current_id);
        let generation = tray.tracking.borrow().generation;
        tray.tracking.borrow_mut().finish(generation);
        println!(
            "PASS: actual menu tracking retains visible actions and completion; latest deferred structure applies after close without another backend event; unrelated menus and reverted structure are safe"
        );
    }

    pub fn verify_animation() {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{
            NSApplication, NSApplicationActivationPolicy, NSEventTrackingRunLoopMode,
        };
        use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop, NSTimer};
        use std::time::{Duration, Instant};
        use tray_icon::menu::ContextMenu;

        let marker = MainThreadMarker::new().unwrap();
        let app = NSApplication::sharedApplication(marker);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        let started = Instant::now();
        let bound = TrayService::new().unwrap().bind(|_| {});
        let precache_ms = started.elapsed().as_millis();
        let tray = bound.get(marker);
        let status = tray.tray.ns_status_item().unwrap();
        let button = status.button(marker).unwrap();
        let initial_button_size = button.frame().size;
        let current_image = || {
            let image = button.image().unwrap();
            assert!(image.isTemplate());
            assert_eq!(image.size().width, 18.0);
            assert_eq!(image.size().height, 18.0);
            assert_eq!(button.frame().size, initial_button_size);
            image.TIFFRepresentation().unwrap().to_vec()
        };
        let spin = |milliseconds| {
            let until = Instant::now() + Duration::from_millis(milliseconds);
            while Instant::now() < until {
                unsafe {
                    NSRunLoop::mainRunLoop().runMode_beforeDate(
                        NSDefaultRunLoopMode,
                        &NSDate::dateWithTimeIntervalSinceNow(0.005),
                    );
                }
            }
        };
        let output = std::env::var_os("VERGE_TRAY_PROBE_OUTPUT").map(std::path::PathBuf::from);
        if let Some(output) = &output {
            std::fs::create_dir_all(output).unwrap();
        }
        for indicator in [
            TrayIndicator::None,
            TrayIndicator::SystemProxy,
            TrayIndicator::Tun,
        ] {
            let snapshot = TraySnapshot {
                indicator,
                ..Default::default()
            };
            tray.update(&snapshot).unwrap();
            let original = current_image();
            for frame in Frame::all() {
                tray.set_icon(indicator, frame).unwrap();
                current_image();
                if let Some(output) = &output {
                    icon_image(indicator, frame)
                        .unwrap()
                        .save(output.join(format!("{indicator:?}-{frame:?}.png")))
                        .unwrap();
                }
            }
            tray.set_icon(indicator, Frame::REST).unwrap();
            assert_eq!(current_image(), original);

            TrayService::animate_click(&bound);
            spin(280);
            let reduced = objc2_app_kit::NSWorkspace::sharedWorkspace()
                .accessibilityDisplayShouldReduceMotion();
            assert_eq!(current_image() == original, reduced);
            // Same-state traffic updates must not interrupt the current pose.
            let pose = current_image();
            tray.update(&snapshot).unwrap();
            assert_eq!(current_image(), pose);
            spin(560);
            assert_eq!(current_image(), original);
        }

        // Old timers cannot overwrite a newly confirmed state or a newer click.
        TrayService::animate_click(&bound);
        spin(200);
        tray.update(&TraySnapshot {
            indicator: TrayIndicator::SystemProxy,
            ..Default::default()
        })
        .unwrap();
        let system = current_image();
        spin(650);
        assert_eq!(current_image(), system);
        let old = tray.start_click_animation().unwrap();
        let new = tray.start_click_animation().unwrap();
        assert_ne!(old, new);
        let peak = Frame::at_elapsed(280);
        tray.advance_click_animation(new, peak);
        let pose = current_image();
        tray.advance_click_animation(old, Frame::REST);
        assert_eq!(current_image(), pose);
        TrayService::animate_click(&bound);
        spin(300);
        TrayService::animate_click(&bound);
        spin(500);
        let reduced =
            objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        assert_eq!(current_image() == system, reduced);
        spin(340);
        assert_eq!(current_image(), system);

        // Exercise the real performClick nested menu loop, not only a synthetic mode pump.
        let menu = unsafe {
            objc2::rc::Retained::retain(
                tray.menu.borrow().ns_menu().cast::<objc2_app_kit::NSMenu>(),
            )
            .unwrap()
        };
        let tracking_samples = Arc::new(Mutex::new(Vec::new()));
        let samples = tracking_samples.clone();
        let active = Arc::new(dispatch2::MainThreadBound::new(
            (bound.clone(), menu.clone()),
            marker,
        ));
        let started = Instant::now();
        let block = block2::RcBlock::new(move |timer: std::ptr::NonNull<NSTimer>| {
            let marker = MainThreadMarker::new().unwrap();
            let (bound, menu) = active.get(marker);
            let image = bound
                .get(marker)
                .tray
                .ns_status_item()
                .unwrap()
                .button(marker)
                .unwrap()
                .image()
                .unwrap();
            assert!(image.isTemplate());
            assert_eq!(image.size().width, 18.0);
            samples
                .lock()
                .unwrap()
                .push(image.TIFFRepresentation().unwrap().to_vec());
            if started.elapsed() >= Duration::from_millis(900) {
                unsafe { timer.as_ref() }.invalidate();
                menu.cancelTrackingWithoutAnimation();
            }
        });
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(0.08, true, &block) };
        unsafe {
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSEventTrackingRunLoopMode);
        }
        TrayService::animate_click(&bound);
        status.setMenu(Some(&menu));
        unsafe {
            button.performClick(None);
        }
        status.setMenu(None);
        let samples = tracking_samples.lock().unwrap();
        assert!(samples.len() >= 3);
        assert_eq!(samples.iter().any(|sample| sample != &system), !reduced);
        assert_eq!(samples.last().unwrap(), &system);
        assert_eq!(current_image(), system);
        println!(
            "PASS: 54 cached frames retain 32px artwork, 18pt templated NSImage and status-button size; all states restore exactly; traffic, state cancellation, stale frames, repeat click and real menu tracking verified; reduced_motion={reduced}; precache_ms={precache_ms}"
        );
    }
}

fn main() {
    if std::env::args().any(|arg| arg == "--animation") {
        objc2::rc::autoreleasepool(|_| native::verify_animation());
    } else if std::env::args().any(|arg| arg == "--tracking") {
        objc2::rc::autoreleasepool(|_| native::verify_tracking_refresh());
    } else {
        objc2::rc::autoreleasepool(|_| native::verify());
        // muda's global MenuEvent handler is a OnceLock: each fixture needs its own process.
        assert!(
            std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--tracking")
                .status()
                .unwrap()
                .success()
        );
        assert!(
            std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--animation")
                .status()
                .unwrap()
                .success()
        );
    }
}
