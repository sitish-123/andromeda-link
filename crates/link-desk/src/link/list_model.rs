use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QList, QModelIndex, QVariant};
use std::pin::Pin;

use super::qobject;

/// Qt's `UserRole`, the first role a model may define.
const USER_ROLE: i32 = 0x0100;

#[derive(Default)]
pub struct ListModelRust {
    pub roles: &'static [&'static str],
    rows: Vec<Vec<QVariant>>,
}

impl qobject::ListModel {
    pub(super) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let row = usize::try_from(index.row()).ok().and_then(|row| self.rows.get(row));
        let cell = usize::try_from(role - USER_ROLE).ok().and_then(|role| row?.get(role));
        cell.cloned().unwrap_or_default()
    }

    pub(super) fn role_names(&self) -> QHash<cxx_qt_lib::QHashPair_i32_QByteArray> {
        let mut names = QHash::default();
        for (role, name) in (USER_ROLE..).zip(self.roles) {
            names.insert(role, QByteArray::from(*name));
        }
        names
    }

    pub(super) fn row_count(&self, _parent: &QModelIndex) -> i32 {
        i32::try_from(self.rows.len()).unwrap_or(i32::MAX)
    }

    pub fn replace(mut self: Pin<&mut Self>, rows: Vec<Vec<QVariant>>) {
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows = rows;
        self.end_reset_model();
    }
}

impl qobject::ListModel {
    /// Changes one cell in place, so a view keeps its scroll position.
    pub fn set(mut self: Pin<&mut Self>, row: usize, role: &str, value: QVariant) {
        let Some(column) = self.roles.iter().position(|name| *name == role) else { return };
        let mut state = self.as_mut().rust_mut();
        let Some(cell) = state.rows.get_mut(row).and_then(|cells| cells.get_mut(column)) else { return };
        *cell = value;
        let index = self.index(i32::try_from(row).unwrap_or(i32::MAX), 0, &QModelIndex::default());
        let mut roles = QList::default();
        roles.append(USER_ROLE + i32::try_from(column).unwrap_or(0));
        self.data_changed(&index, &index, &roles);
    }
}

pub(super) const NOTIFICATION_ROLES: &[&str] = &["key", "app", "title", "body", "ageText", "iconUrl", "canReply"];
pub(super) const RECENT_ROLES: &[&str] = &["name", "detail", "ageText", "thumbUrl", "kind"];
