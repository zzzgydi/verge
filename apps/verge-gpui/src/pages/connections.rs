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

use crate::{
    format,
    i18n::{self, Lang, tr},
    view::MainView,
};

use super::{muted, page_title};

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

/// 连接表的 delegate：持有快照数据，右键菜单和按钮通过 channel 回到 MainView dispatch。
pub struct ConnectionsDelegate {
    pub connections: Vec<Connection>,
    columns: Vec<Column>,
    actions: mpsc::Sender<UiAction>,
    language: Lang,
}

impl ConnectionsDelegate {
    pub fn new(actions: mpsc::Sender<UiAction>) -> Self {
        // 创建时设置尚未到达，列名先用英文；语言确定后由 render 里的 set_language 重设。
        let language = Lang::En;
        Self {
            connections: Vec::new(),
            // 7 列总宽控制在内容区（约 850px）内，避免横向挤压。
            columns: vec![
                Column::new("process", tr(language, COLUMN_KEYS[0])).width(110.),
                Column::new("target", tr(language, COLUMN_KEYS[1])).width(170.),
                Column::new("rule", tr(language, COLUMN_KEYS[2])).width(130.),
                Column::new("chains", tr(language, COLUMN_KEYS[3])).width(140.),
                Column::new("upload", tr(language, COLUMN_KEYS[4])).width(80.).text_right(),
                Column::new("download", tr(language, COLUMN_KEYS[5])).width(80.).text_right(),
                Column::new("actions", tr(language, COLUMN_KEYS[6])).width(60.),
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
                Button::new(SharedString::from(format!("close-conn-{}", connection.id)))
                    .label(tr(self.language, "connections.close"))
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
        menu.item(
            PopupMenuItem::new(tr(self.language, "connections.close_menu")).on_click(
                move |_, _, _| {
                    let _ = actions.send(UiAction::CloseConnection(id.clone()));
                },
            ),
        )
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let snapshot: Option<ConnectionSnapshot> = view.state.connections.clone();
    // 快照内容没变就不 refresh，避免每帧重建表格；语言切换（列名变化）也要 refresh。
    let connections = snapshot
        .as_ref()
        .map(|snapshot| snapshot.connections.clone())
        .unwrap_or_default();
    view.connections_table.update(cx, |table, cx| {
        let mut dirty = table.delegate_mut().set_language(lang);
        if table.delegate().connections != connections {
            table.delegate_mut().connections = connections;
            dirty = true;
        }
        if dirty {
            table.refresh(cx);
        }
    });

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
        .size_full()
        .gap_3()
        .child(
            h_flex()
                .justify_between()
                .child(page_title(tr(lang, "connections.title")))
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
