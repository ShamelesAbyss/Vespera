//! The Store manages updates for this package.

use bevy::prelude::Resource;

pub(super) const STORE_BUILD: bool = true;

#[derive(Resource)]
pub(super) struct Updates {
    pub status: String,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            status: "Updates are managed by Microsoft Store.".into(),
        }
    }
}

impl Updates {
    pub(super) fn check(&mut self) {}

    pub(super) fn poll(&mut self) {}

    pub(super) fn install_label(&self) -> Option<&'static str> {
        None
    }

    pub(super) fn install(&mut self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_build_offers_no_standalone_update_action() {
        let mut updates = Updates::default();
        updates.check();
        assert_eq!(updates.status, "Updates are managed by Microsoft Store.");
        assert_eq!(updates.install_label(), None);
        assert!(!updates.install());
    }
}
