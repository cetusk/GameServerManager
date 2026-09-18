//! Route changes are deferred until the user explicitly discards an open editor.
use super::*;
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Destination {
    Game(String),
    Overview,
    Choose,
    Page(i32),
    Preferences,
    Create(String),
    Reload,
    ChangeDirectory,
    Close,
}
fn defer(
    pending: &mut Option<Destination>,
    editing: bool,
    destination: Destination,
) -> Option<Destination> {
    if pending.is_some() {
        return None;
    }
    if editing {
        *pending = Some(destination);
        None
    } else {
        Some(destination)
    }
}
impl Session {
    pub(super) fn can_change_directory(&self) -> bool {
        !self.registration.busy()
            && !self.file_picker_open
            && self.selection_receiver.is_none()
            && self.transfer_receiver.is_none()
            && self.import_receiver.is_none()
            && self.copy_receiver.is_none()
            && self.editor_receiver.is_none()
            && self.review_receiver.is_none()
            && self.save_job.is_none()
            && !self.creation_busy()
            && self
                .app
                .snapshot()
                .instances
                .iter()
                .all(|v| v.active.is_none() && v.observation.process == ProcessState::Absent)
    }
    pub(super) fn creation_busy(&self) -> bool {
        self.creation_receiver.is_some() || (self.creation.is_some() && self.registration.busy())
    }
    pub(super) fn editing(&self) -> bool {
        self.editor_receiver.is_some()
            || self.review_receiver.is_some()
            || self
                .active
                .and_then(|id| {
                    self.app
                        .snapshot()
                        .instances
                        .iter()
                        .find(|v| v.instance.id == id)
                        .map(|v| v.instance.game_id.to_string())
                })
                .is_some_and(|game| self.drafts.contains_key(&(game, self.page == 2)))
            || self.creation.is_some()
    }
    pub(super) fn navigate(&mut self, destination: Destination, ui: &MainWindow) {
        if self.pending_navigation.is_some()
            || self.pending_restore.is_some()
            || ui.global::<ConfigEditor>().get_confirming()
        {
            return;
        }
        let same = match &destination {
            Destination::Page(page) => *page == self.page,
            Destination::Game(game) => {
                !self.overview
                    && !self.choosing
                    && !self.preferences_open
                    && ui.get_active_game().as_str() == game
            }
            _ => false,
        };
        if same {
            return;
        }
        let editing = self.editing();
        if let Some(destination) = defer(&mut self.pending_navigation, editing, destination) {
            self.finish_navigation(destination, ui);
        } else {
            ui.set_leave_visible(true);
            ui.invoke_focus_leave_dialog();
        }
    }
    pub(super) fn finish_navigation(&mut self, destination: Destination, ui: &MainWindow) {
        match destination {
            Destination::Game(game) => self.select_game(&game),
            Destination::Overview => {
                self.cancel_selection();
                self.overview = true;
                self.preferences_open = false;
            }
            Destination::Choose => {
                self.draft = self
                    .app
                    .snapshot()
                    .config
                    .enabled_games
                    .into_iter()
                    .collect();
                self.query.clear();
                self.choosing = true;
                self.preferences_open = false;
                self.error.clear();
                ui.set_search_text("".into());
            }
            Destination::Page(page) => self.page = page,
            Destination::Preferences => self.preferences_open = !self.preferences_open,
            Destination::Create(game) => {
                self.creation = Some(crate::creation::Draft::new(&game));
                self.page = 1;
                ui.global::<ConfigEditor>().set_message("".into());
            }
            Destination::ChangeDirectory => {
                if !self.can_change_directory() {
                    return;
                }
                self.change_directory = true;
                let _ = slint::quit_event_loop();
            }
            Destination::Reload => {
                self.reload = true;
                let _ = slint::quit_event_loop();
            }
            Destination::Close => {
                let _ = slint::quit_event_loop();
            }
        }
        self.remember_navigation();
    }
    pub(super) fn discard_and_leave(&mut self, ui: &MainWindow) {
        if self.editor_receiver.is_some()
            || self.review_receiver.is_some()
            || self.save_job.is_some()
            || self.creation_busy()
        {
            return;
        }
        if let Some(destination) = self.pending_navigation.take() {
            self.drafts.clear();
            self.creation = None;
            self.pending_creation = None;
            self.pending_edit = None;
            ui.global::<ConfigEditor>().set_confirming(false);
            ui.global::<ConfigEditor>().set_message("".into());
            ui.set_leave_visible(false);
            self.finish_navigation(destination, ui);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_open_editor_blocks_all_routes_until_an_explicit_decision() {
        for destination in [
            Destination::Page(2),
            Destination::Game("conan".into()),
            Destination::Overview,
            Destination::Choose,
            Destination::Preferences,
            Destination::Reload,
            Destination::ChangeDirectory,
            Destination::Close,
        ] {
            let mut pending = None;
            assert!(defer(&mut pending, true, destination.clone()).is_none());
            assert_eq!(pending, Some(destination.clone()));
            // Repeated clicks cannot change the original requested destination.
            assert!(defer(&mut pending, true, Destination::Page(4)).is_none());
            assert_eq!(pending.take(), Some(destination));
            // Keeping the editor open means the next attempt also requires a decision.
            assert!(defer(&mut pending, true, Destination::Close).is_none());
            pending = None;
            assert_eq!(
                defer(&mut pending, false, Destination::Close),
                Some(Destination::Close)
            );
        }
    }
}
