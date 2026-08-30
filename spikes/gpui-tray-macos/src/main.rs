mod command;
mod tray;

use std::sync::mpsc;

use command::AppCommand;
use gpui::*;
use gpui_component::{Root, button::*, *};
use tray::TrayService;

struct MainView;

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap_3()
            .size_full()
            .items_center()
            .justify_center()
            .child("Verge GPUI tray spike")
            .child(
                Button::new("close-hint")
                    .primary()
                    .label("Use the tray menu to drive commands")
                    .on_click(|_, _, _| {}),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx| {
        gpui_component::init(cx);

        let (command_tx, command_rx) = mpsc::channel();
        let tray = TrayService::new(move |command| {
            let _ = command_tx.send(command);
        })
        .expect("failed to initialize tray");

        cx.spawn(async move |cx| {
            let _tray = tray;
            let window = cx
                .open_window(WindowOptions::default(), |window, cx| {
                    let view = cx.new(|_| MainView);
                    cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
                })
                .expect("failed to open window");

            loop {
                while let Ok(command) = command_rx.try_recv() {
                    match command {
                        AppCommand::ShowMainWindow => {
                            eprintln!("tray command: show main window");
                            cx.update(|cx| {
                                cx.activate(true);
                                let _ = window.update(cx, |_, window, _| window.activate_window());
                            });
                        }
                        AppCommand::HideMainWindow => {
                            eprintln!("tray command: hide main window");
                            cx.update(|cx| cx.hide());
                        }
                        AppCommand::Quit => {
                            eprintln!("tray command: quit");
                            cx.update(|cx| cx.quit());
                            return;
                        }
                    }
                }

                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
            }
        })
        .detach();
    });
}
