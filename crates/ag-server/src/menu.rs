//! Server menu and CLI status commands.
//! Fulfills H12: name, Settings, Plugins, Statistics, About, Quit.

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMenuItem {
    Name(String),
    Settings,
    Plugins,
    Statistics,
    About,
    Quit,
}

#[derive(Debug, Clone)]
pub struct ServerMenu {
    pub name: String,
    pub id: u16,
    pub version: String,
}

impl ServerMenu {
    pub fn new(name: impl Into<String>, id: u16, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            id,
            version: version.into(),
        }
    }

    pub fn items(&self) -> Vec<ServerMenuItem> {
        let name_str = if self.id > 0 {
            format!("{}:{}", self.name, self.id)
        } else {
            self.name.clone()
        };
        vec![
            ServerMenuItem::Name(name_str),
            ServerMenuItem::Settings,
            ServerMenuItem::Plugins,
            ServerMenuItem::Statistics,
            ServerMenuItem::About,
            ServerMenuItem::Quit,
        ]
    }

    pub fn render_cli(&self) -> String {
        let mut out = String::new();
        writeln!(&mut out, "AudioGridder Server Menu:").unwrap();
        writeln!(&mut out, "1. Name: {}", if self.id > 0 { format!("{}:{}", self.name, self.id) } else { self.name.clone() }).unwrap();
        writeln!(&mut out, "2. Settings").unwrap();
        writeln!(&mut out, "3. Plugins").unwrap();
        writeln!(&mut out, "4. Statistics").unwrap();
        writeln!(&mut out, "5. About (v{})", self.version).unwrap();
        writeln!(&mut out, "6. Quit").unwrap();
        out
    }

    pub fn handle_action(&self, item: ServerMenuItem) -> &'static str {
        match item {
            ServerMenuItem::Name(_) => "server_name",
            ServerMenuItem::Settings => "open_settings",
            ServerMenuItem::Plugins => "open_plugins",
            ServerMenuItem::Statistics => "open_statistics",
            ServerMenuItem::About => "open_about",
            ServerMenuItem::Quit => "quit",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h12_server_menu_and_cli_status_commands() {
        let menu = ServerMenu::new("Studio-Main", 0, "0.1.0-alpha");
        let items = menu.items();
        assert_eq!(items.len(), 6);
        assert_eq!(items[0], ServerMenuItem::Name("Studio-Main".to_string()));
        assert_eq!(items[1], ServerMenuItem::Settings);
        assert_eq!(items[2], ServerMenuItem::Plugins);
        assert_eq!(items[3], ServerMenuItem::Statistics);
        assert_eq!(items[4], ServerMenuItem::About);
        assert_eq!(items[5], ServerMenuItem::Quit);

        let menu_with_id = ServerMenu::new("Studio-Worker", 2, "0.1.0-alpha");
        assert_eq!(menu_with_id.items()[0], ServerMenuItem::Name("Studio-Worker:2".to_string()));

        let text = menu.render_cli();
        assert!(text.contains("Settings"));
        assert!(text.contains("Plugins"));
        assert!(text.contains("Statistics"));
        assert!(text.contains("About"));
        assert!(text.contains("Quit"));

        assert_eq!(menu.handle_action(ServerMenuItem::Quit), "quit");
        assert_eq!(menu.handle_action(ServerMenuItem::Settings), "open_settings");
        assert_eq!(menu.handle_action(ServerMenuItem::Plugins), "open_plugins");
        assert_eq!(menu.handle_action(ServerMenuItem::Statistics), "open_statistics");
        assert_eq!(menu.handle_action(ServerMenuItem::About), "open_about");
    }
}
