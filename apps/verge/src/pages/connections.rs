use std::sync::Arc;

use crate::domain::{Connection, ConnectionSnapshot};
use crate::ui::UiAction;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogButtonProps,
    menu::{PopupMenu, PopupMenuItem},
    scroll::ScrollableElement as _,
    table::{Column, DataTable, TableDelegate, TableState},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::*;

use crate::{
    format,
    i18n::{self, Lang, tr},
    view::MainView,
};

use super::components::PageHeader;
use super::muted;

/// 列名对应的 i18n key（与 columns 的声明顺序一致）。
const COLUMN_KEYS: [&str; 7] = [
    "connections.col.process",
    "connections.col.target",
    "connections.col.rule",
    "connections.col.chains",
    "connections.col.upload",
    "connections.col.download",
    "connections.col.actions",
];

/// 连接表共享不可变快照；按钮通过弱实体句柄即时派发 typed action。
pub struct ConnectionsDelegate {
    pub snapshot: Option<Arc<ConnectionSnapshot>>,
    columns: Vec<Column>,
    actions: WeakEntity<MainView>,
    language: Lang,
}

impl ConnectionsDelegate {
    pub fn new(actions: WeakEntity<MainView>) -> Self {
        // 创建时使用英文；设置响应到达后同步列名。
        let language = Lang::En;
        Self {
            snapshot: None,
            // Keep all seven columns, including close actions, visible at 960px window width.
            columns: vec![
                Column::new("process", tr(language, COLUMN_KEYS[0])).width(80.),
                Column::new("target", tr(language, COLUMN_KEYS[1])).width(160.),
                Column::new("rule", tr(language, COLUMN_KEYS[2])).width(100.),
                Column::new("chains", tr(language, COLUMN_KEYS[3])).width(100.),
                Column::new("upload", tr(language, COLUMN_KEYS[4]))
                    .width(80.)
                    .text_right(),
                Column::new("download", tr(language, COLUMN_KEYS[5]))
                    .width(80.)
                    .text_right(),
                Column::new("actions", tr(language, COLUMN_KEYS[6])).width(48.),
            ],
            actions,
            language,
        }
    }

    /// 语言切换后重设列名；返回是否有变化（调用方据此 refresh 表格）。
    pub fn set_language(&mut self, language: Lang) -> bool {
        if self.language == language {
            return false;
        }
        self.language = language;
        for (column, key) in self.columns.iter_mut().zip(COLUMN_KEYS) {
            column.name = tr(language, key).into();
        }
        true
    }
}

impl TableDelegate for ConnectionsDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.snapshot.as_ref().map_or(0, |s| s.connections.len())
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(connection) = self
            .snapshot
            .as_ref()
            .and_then(|s| s.connections.get(row_ix))
        else {
            return div().into_any_element();
        };
        let text = |content: String| {
            div()
                .text_sm()
                .overflow_x_hidden()
                .truncate()
                .child(content)
                .into_any_element()
        };
        match col_ix {
            0 => text(if connection.process.is_empty() {
                tr(self.language, "connections.unknown_process").into()
            } else {
                connection.process.clone()
            }),
            1 => {
                let target = if connection.host.is_empty() {
                    connection.destination.clone()
                } else {
                    connection.host.clone()
                };
                // 截断的目标悬停可见完整内容。
                div()
                    .id(SharedString::from(format!("conn-target-{}", connection.id)))
                    .text_sm()
                    .overflow_x_hidden()
                    .truncate()
                    .child(target.clone())
                    .tooltip(move |window, cx| Tooltip::new(target.clone()).build(window, cx))
                    .on_mouse_down(MouseButton::Left, {
                        let actions = self.actions.clone();
                        let connection = connection.clone();
                        move |_, window, cx| {
                            let _ = actions.update(cx, |view, cx| {
                                view.open_connection_details(connection.clone(), window, cx)
                            });
                        }
                    })
                    .into_any_element()
            }
            2 => text(if connection.rule_payload.is_empty() {
                connection.rule.clone()
            } else {
                format!("{} {}", connection.rule, connection.rule_payload)
            }),
            3 => text(if connection.chains.is_empty() {
                tr(self.language, "connections.direct").into()
            } else {
                connection.chains.join(" → ")
            }),
            4 => div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(format::bytes(connection.upload))
                .into_any_element(),
            5 => div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(format::bytes(connection.download))
                .into_any_element(),
            6 => {
                let id = connection.id.clone();
                let actions = self.actions.clone();
                div()
                    .debug_selector(move || format!("close-connection-{row_ix}"))
                    .child(
                        Button::new(SharedString::from(format!("close-conn-{}", connection.id)))
                            .label(tr(self.language, "connections.close"))
                            .xsmall()
                            .ghost()
                            .on_click(move |_, _, cx| {
                                let _ = actions.update(cx, |view, cx| {
                                    view.dispatch(UiAction::CloseConnection(id.clone()), cx)
                                });
                            }),
                    )
                    .into_any_element()
            }
            _ => div().into_any_element(),
        }
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let Some(connection) = self
            .snapshot
            .as_ref()
            .and_then(|s| s.connections.get(row_ix))
        else {
            return menu;
        };
        let id = connection.id.clone();
        let actions = self.actions.clone();
        let details = connection.clone();
        let details_actions = self.actions.clone();
        menu.item(
            PopupMenuItem::new(tr(self.language, "connections.details")).on_click(
                move |_, window, cx| {
                    let _ = details_actions.update(cx, |view, cx| {
                        view.open_connection_details(details.clone(), window, cx)
                    });
                },
            ),
        )
        .item(
            PopupMenuItem::new(tr(self.language, "connections.close_menu")).on_click(
                move |_, _, cx| {
                    let _ = actions.update(cx, |view, cx| {
                        view.dispatch(UiAction::CloseConnection(id.clone()), cx)
                    });
                },
            ),
        )
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let summary = view.state.connections.as_ref().map_or_else(
        || tr(lang, "connections.waiting").to_owned(),
        |connections| {
            i18n::fmt_connections_summary(
                lang,
                connections.connection_count,
                &format::bytes(connections.upload_total),
                &format::bytes(connections.download_total),
            )
        },
    );

    v_flex()
        .flex_1()
        .min_h_0()
        .gap_4()
        .child(
            PageHeader::new(tr(lang, "connections.title")).child(
                Button::new("close-all-connections")
                    .label(tr(lang, "connections.close_all"))
                    .small()
                    .danger()
                    .disabled(
                        view.state.connection_notice.is_some()
                            || view.is_pending(&["connections_write"])
                            || view
                                .state
                                .connections
                                .as_ref()
                                .is_none_or(|s| s.connection_count == 0),
                    )
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.confirm_close_all_connections(window, cx)
                    })),
            ),
        )
        .child(muted(summary, cx))
        .child(
            div().flex_1().min_h_0().child(
                DataTable::new(&view.connections_table)
                    .small()
                    .bordered(false)
                    .stripe(true),
            ),
        )
        .into_any_element()
}

impl MainView {
    pub(crate) fn open_connection_details(
        &mut self,
        connection: Connection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lang = self.lang();
        // Capture the selected row: live snapshots must not change the detail being inspected.
        let rows = vec![
            ("ID", connection.id),
            (tr(lang, "connections.col.process"), connection.process),
            (tr(lang, "connections.network"), connection.network),
            (tr(lang, "connections.source"), connection.source),
            (tr(lang, "connections.col.target"), connection.destination),
            ("Host", connection.host),
            (tr(lang, "connections.started"), connection.start),
            (
                tr(lang, "connections.col.rule"),
                format!("{} {}", connection.rule, connection.rule_payload),
            ),
            (
                tr(lang, "connections.col.chains"),
                connection.chains.join(" → "),
            ),
            (
                tr(lang, "connections.col.upload"),
                format::bytes(connection.upload),
            ),
            (
                tr(lang, "connections.col.download"),
                format::bytes(connection.download),
            ),
        ];
        window.open_sheet(cx, move |sheet, _, cx| {
            sheet
                .title(tr(lang, "connections.details"))
                .size(rems(30.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .gap_3()
                        .overflow_y_scrollbar()
                        .children(rows.iter().map(|(label, value)| {
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(*label),
                                )
                                .child(div().text_sm().child(if value.trim().is_empty() {
                                    "—".into()
                                } else {
                                    value.clone()
                                }))
                        })),
                )
        });
    }

    pub(crate) fn confirm_close_all_connections(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lang = self.lang();
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .confirm()
                .title(tr(lang, "connections.close_all"))
                .description(tr(lang, "connections.close_all.desc"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr(lang, "connections.close_all"))
                        .cancel_text(tr(lang, "common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |view, cx| {
                        view.dispatch_confirmed(UiAction::CloseAllConnections, cx)
                    });
                    true
                })
        });
    }
}
