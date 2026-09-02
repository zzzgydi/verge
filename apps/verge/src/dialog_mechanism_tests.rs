//! Root / Dialog / Sheet 机制的 UI 测试。
//!
//! 用 gpui 的 `add_window_view`（与 gpui-component 官方测试一致的环境）验证：
//! 1. dialog/sheet 打开后 `Root::render_*_layer` 必须存在（MainView 已挂载
//!    overlay 层——此前未挂载导致"点了没反应"）；
//! 2. sheet builder 渲染时不能读 MainView 实体（已用 SheetState 解耦）。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use gpui::{AppContext as _, Styled as _, TestAppContext, VisualTestContext};
use gpui_component::{ActiveTheme as _, Root, WindowExt as _};

use crate::i18n::Lang;
use crate::view::MainView;

type ViewHolder = Rc<RefCell<Option<gpui::Entity<MainView>>>>;

/// 创建窗口：MainView + Root（与真实 main() 同构）。
fn setup(cx: &mut TestAppContext) -> (gpui::Entity<Root>, ViewHolder, &mut VisualTestContext) {
    cx.update(gpui_component::init);
    let (request_tx, _request_rx) = mpsc::channel();
    let view_holder: ViewHolder = Default::default();
    let holder = view_holder.clone();
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(request_tx, window, cx));
        *holder.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx).bg(cx.theme().background)
    });
    (root, view_holder, cx)
}

#[gpui::test]
fn import_dialog_opens_and_renders(cx: &mut TestAppContext) {
    let (_, view_holder, cx) = setup(cx);
    let view = view_holder
        .borrow()
        .clone()
        .expect("view should be created");

    // 直接调用按钮回调路径（VisualTestContext 上下文，不经 WindowHandle::update，
    // 避免无谓的 root lease）。
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.open_import_dialog(window, cx));
    });
    cx.run_until_parked();

    let (has_dialog, layer) = cx.update(|window, cx| {
        (
            window.has_active_dialog(cx),
            Root::render_dialog_layer(window, cx).is_some(),
        )
    });
    assert!(has_dialog, "open_import_dialog 后应有活跃 dialog");
    assert!(
        layer,
        "dialog 打开后渲染层必须存在（MainView 已挂载 overlay）"
    );
}

#[gpui::test]
fn yaml_sheet_opens_and_renders(cx: &mut TestAppContext) {
    let (_, view_holder, cx) = setup(cx);
    let view = view_holder
        .borrow()
        .clone()
        .expect("view should be created");

    let id = crate::domain::ProfileId::parse("demo").unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.open_yaml_sheet(id, window, cx));
    });
    cx.run_until_parked();

    let (has_sheet, layer) = cx.update(|window, cx| {
        (
            window.has_active_sheet(cx),
            Root::render_sheet_layer(window, cx).is_some(),
        )
    });
    assert!(has_sheet, "open_yaml_sheet 后应有活跃 sheet");
    assert!(
        layer,
        "sheet 打开后渲染层必须存在（MainView 已挂载 overlay）"
    );
}

#[gpui::test]
fn merge_sheet_opens_and_renders_without_reading_main_view(cx: &mut TestAppContext) {
    let (_, view_holder, cx) = setup(cx);
    let view = view_holder
        .borrow()
        .clone()
        .expect("view should be created");

    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.open_merge_sheet(window, cx));
    });
    // 渲染（sheet builder 执行）；此前 builder 读 MainView 会 double-lease panic，
    // 现在状态在 SheetState 实体里，渲染必须无 panic 且渲染层存在。
    cx.run_until_parked();

    let (has_sheet, layer) = cx.update(|window, cx| {
        (
            window.has_active_sheet(cx),
            Root::render_sheet_layer(window, cx).is_some(),
        )
    });
    assert!(has_sheet && layer, "merge sheet 应打开且渲染层存在");
}

#[gpui::test]
fn merged_sheet_opens_and_renders(cx: &mut TestAppContext) {
    let (_, view_holder, cx) = setup(cx);
    let view = view_holder
        .borrow()
        .clone()
        .expect("view should be created");

    let id = crate::domain::ProfileId::parse("demo").unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.open_merged_sheet(id, window, cx));
    });
    cx.run_until_parked();

    let (has_sheet, layer) = cx.update(|window, cx| {
        (
            window.has_active_sheet(cx),
            Root::render_sheet_layer(window, cx).is_some(),
        )
    });
    assert!(has_sheet && layer, "merged sheet 应打开且渲染层存在");
}

/// 语言切换：设置写入后 MainView::lang 立即反映新语言，sync_form_inputs
/// 同步重设输入框占位（状态驱动，无需重启；占位内容正确性由 i18n 查表测试覆盖）。
#[gpui::test]
fn language_switch_updates_view_language(cx: &mut TestAppContext) {
    use crate::domain::{ApplicationSettings, ApplicationSettingsSnapshot};

    let (_, view_holder, cx) = setup(cx);
    let view = view_holder
        .borrow()
        .clone()
        .expect("view should be created");

    // 设置未加载时回退英文（与 domain 默认语言一致）。
    cx.update(|_, cx| {
        view.update(cx, |view, _| assert_eq!(view.lang(), Lang::En));
    });

    // 写入 zh-CN 设置并同步表单：语言随即切换。
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.application_settings = Some(ApplicationSettingsSnapshot {
                settings: ApplicationSettings {
                    language: "zh-CN".into(),
                    ..Default::default()
                },
                data_directory: "/tmp/verge-i18n-test".into(),
                app_version: None,
            });
            view.sync_form_inputs(window, cx);
            assert_eq!(view.lang(), Lang::ZhCn);
        });
    });

    // 切回英文同样即时生效。
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(snapshot) = view.state.application_settings.as_mut() {
                snapshot.settings.language = "en".into();
            }
            view.sync_form_inputs(window, cx);
            assert_eq!(view.lang(), Lang::En);
        });
    });
}
