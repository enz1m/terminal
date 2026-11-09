//! Terminal test accessor methods.

use super::*;

impl App {
    /// The id of the session at `index`, if any.
    pub(crate) fn session_id_at(&self, index: usize) -> Option<String> {
        self.sessions.get(index).map(|s| s.id.clone())
    }

    /// The title of the session at `index`, if any.
    #[cfg(test)]
    pub(crate) fn session_title_at(&self, index: usize) -> Option<String> {
        self.sessions.get(index).map(|s| s.title.clone())
    }

    /// Whether the session at `index` is pinned.
    #[cfg(test)]
    pub(crate) fn session_pinned_at(&self, index: usize) -> Option<bool> {
        self.sessions.get(index).map(|s| s.pinned)
    }

    /// Whether the session at `index` is detached.
    #[cfg(test)]
    pub(crate) fn session_detached_at(&self, index: usize) -> Option<bool> {
        self.sessions.get(index).map(|s| s.detached)
    }

    /// How many sessions there are.
    #[cfg(test)]
    pub(crate) fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// The in-progress session-rename draft, if any.
    #[cfg(test)]
    pub(crate) fn session_rename_draft(&self) -> Option<String> {
        self.session_rename.as_ref().map(|(_, d)| d.clone())
    }
}
