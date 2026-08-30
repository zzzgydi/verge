use std::sync::mpsc;

use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{PopupMenu, PopupMenuItem},
    table::{Column, DataTable, TableDelegate, TableState},
    tooltip::Tooltip,
    v_flex,
};
use verge_domain::{Connection, ConnectionSnapshot};
use verge_ui::UiAction;

use crate::{format, view::MainView};

use super::{muted, page_title};

/// 连接表的 delegate：持有快照数据，右键菜单和按钮通过 channel 回到 MainView dispatch。
pub struct ConnectionsDelegate {
    pub connections: Vec<Connection>,
    columns: Vec<Column>,
    actions: mpsc::Sender<UiAction>,
}

impl ConnectionsDelegate {
    pub fn new(actions: mpsc::Sender<UiAction>) -> Self {
        Self {
            connections: Vec::new(),
            columns: vec![
                Column::new("process", "进程").width(160.),
                Column::new("target", "目标").width(240.),
                Column::new("rule", "规则").width(160.),
                Column::new("chains", "链路").width(200.),
                Column::new("upload", "上传").width(90.).text_right(),
                Column::new("download", "下载").width(90.).text_right(),
                Column::new("actions", "操作").width(70.),
            ],
            actions,
        }
    }
}

impl TableDelegate for ConnectionsDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.connections.len()
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
        let Some(connection) = self.connections.get(row_ix) else {
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
                "未知进程".into()
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
                    .into_any_element()
            }
            2 => text(if connection.rule_payload.is_empty() {
                connection.rule.clone()
            } else {
                format!("{} {}", connection.rule, connection.rule_payload)
            }),
            3 => text(if connection.chains.is_empty() {
                "直连".into()
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
                Button::new(SharedString::from(format!("close-conn-{}", connection.id)))
                    .label("关闭")
                    .xsmall()
                    .ghost()
                    .on_click(move |_, _, _| {
                        let _ = actions.send(UiAction::CloseConnection(id.clone()));
                    })
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
        let Some(connection) = self.connections.get(row_ix) else {
            return menu;
        };
        let id = connection.id.clone();
        let actions = self.actions.clone();
        menu.item(PopupMenuItem::new("关闭连接").on_click(move |_, _, _| {
            let _ = actions.send(UiAction::CloseConnection(id.clone()));
        }))
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let snapshot: Option<ConnectionSnapshot> = view.state.connections.clone();
    // 快照内容没变就不 refresh，避免每帧重建表格。
    let connections = snapshot
        .as_ref()
        .map(|snapshot| snapshot.connections.clone())
        .unwrap_or_default();
    view.connections_table.update(cx, |table, cx| {
        if table.delegate().connections != connections {
            table.delegate_mut().connections = connections;
            table.refresh(cx);
        }
    });

    let summary = view.state.connections.as_ref().map_or_else(
        || "等待连接快照…".to_owned(),
        |connections| {
            format!(
                "{} 条活跃连接 · 累计上传 {} · 累计下载 {}",
                connections.connection_count,
                format::bytes(connections.upload_total),
                format::bytes(connections.download_total)
            )
        },
    );

    v_flex()
        .size_full()
        .gap_3()
        .child(
            h_flex()
                .justify_between()
                .child(page_title("连接"))
                .child(muted(summary, cx)),
        )
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
