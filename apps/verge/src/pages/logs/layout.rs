//! Cache dimensions only: unchanged messages are not shaped again on every frame.
use std::collections::VecDeque;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

use super::{LOG_FONT_SIZE, LOG_INSET, LOG_ROW_HEIGHT, display_text};
use crate::ui::UiState;

#[derive(Default)]
pub struct LogLayout {
    font: Option<Font>,
    revision: u64,
    epoch: u64,
    filter_key: Option<(u64, u64, crate::pages::filters::LogFilter)>,
    filtered: std::rc::Rc<Vec<usize>>,
    entries: VecDeque<(u64, Size<Pixels>)>,
}

impl LogLayout {
    pub fn sync(&mut self, state: &UiState, window: &Window, cx: &App) {
        let font = font(cx.theme().mono_font_family.clone());
        if self.font.as_ref() != Some(&font)
            || state.log_revision < self.revision
            || state.log_layout_epoch != self.epoch
        {
            self.entries.clear();
            self.font = Some(font.clone());
        }
        let first = state.log_revision.saturating_sub(state.logs.len() as u64);
        while self.entries.front().is_some_and(|(id, _)| *id < first) {
            self.entries.pop_front();
        }
        let start = self.entries.back().map_or(first, |(id, _)| id + 1);
        for (ix, log) in state.logs.iter().enumerate().skip((start - first) as usize) {
            let text = display_text(log);
            let mut width = px(0.);
            let mut lines = 0;
            for line in text.split('\n') {
                let run = TextRun {
                    len: line.len(),
                    font: font.clone(),
                    ..Default::default()
                };
                width = width.max(
                    window
                        .text_system()
                        .shape_line(line.to_owned().into(), px(LOG_FONT_SIZE), &[run], None)
                        .width(),
                );
                lines += 1;
            }
            self.entries.push_back((
                first + ix as u64,
                size(
                    width.ceil() + px(LOG_INSET * 2.),
                    px(LOG_ROW_HEIGHT * lines as f32),
                ),
            ));
        }
        self.revision = state.log_revision;
        self.epoch = state.log_layout_epoch;
    }

    pub fn filtered(
        &mut self,
        state: &UiState,
        filter: &crate::pages::filters::LogFilter,
    ) -> std::rc::Rc<Vec<usize>> {
        let key = (state.log_revision, state.log_layout_epoch, filter.clone());
        if self.filter_key.as_ref() != Some(&key) {
            self.filtered = std::rc::Rc::new(
                state
                    .logs
                    .iter()
                    .enumerate()
                    .filter(|(_, log)| filter.matches(log))
                    .map(|(ix, _)| ix)
                    .collect(),
            );
            self.filter_key = Some(key);
        }
        self.filtered.clone()
    }

    pub fn size(&self, index: usize) -> Size<Pixels> {
        self.entries[index].1
    }
}
