use std::collections::HashMap;
use std::rc::Rc;

use anyhow::{Error, Result};
use lazy_static::lazy_static;

use radicle_terminal as term;

use term::tui::events::{InputEvent, Key};
use term::tui::store::Store;
use term::tui::theme::Theme;
use term::tui::window::{EmptyWidget, PageWidget, ShortcutWidget, TitleWidget};
use term::tui::{Application, State};

#[derive(Clone, Eq, PartialEq)]
pub enum Action {
    Quit,
}

lazy_static! {
    static ref KEY_BINDINGS: HashMap<Key, Action> =
        [(Key::Char('q'), Action::Quit)].iter().cloned().collect();
}

fn main() -> Result<(), Error> {
    // Create basic application that will call `update` on
    // every input event received from event thread.
    let mut application = Application::new(&on_action).store(vec![
        ("app.shortcuts", Box::new(vec![String::from("q quit")])),
        ("app.title", Box::new(String::from("tui-example"))),
    ]);

    // Create a single-page application
    let pages = vec![PageWidget {
        title: Rc::new(TitleWidget),
        widgets: vec![Rc::new(EmptyWidget)],
        shortcuts: Rc::new(ShortcutWidget),
    }];

    // Use default, borderless theme
    let theme = Theme::default_dark();

    // Run application
    application.execute(pages, &theme)?;
    Ok(())
}

fn on_action(store: &mut Store, event: &InputEvent) -> anyhow::Result<(), anyhow::Error> {
    // Set application set to `State::Exiting` when the key 'q' is received.
    // Note that any special tick handling is ignored for now.
    if let InputEvent::Input(key) = *event {
        if let Some(action) = KEY_BINDINGS.get(&key) {
            match action {
                Action::Quit => store.set("app.state", Box::new(State::Exiting)),
            }
        }
    }
    Ok(())
}