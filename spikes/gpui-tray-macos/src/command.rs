#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppCommand {
    ShowMainWindow,
    HideMainWindow,
    Quit,
}

impl AppCommand {
    pub fn from_menu_id(id: &str) -> Option<Self> {
        match id {
            "show-main-window" => Some(Self::ShowMainWindow),
            "hide-main-window" => Some(Self::HideMainWindow),
            "quit" => Some(Self::Quit),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppCommand;

    #[test]
    fn maps_known_menu_ids_to_commands() {
        assert_eq!(
            AppCommand::from_menu_id("show-main-window"),
            Some(AppCommand::ShowMainWindow)
        );
        assert_eq!(
            AppCommand::from_menu_id("hide-main-window"),
            Some(AppCommand::HideMainWindow)
        );
        assert_eq!(AppCommand::from_menu_id("quit"), Some(AppCommand::Quit));
        assert_eq!(AppCommand::from_menu_id("unknown"), None);
    }
}
