use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QList, QModelIndex, QVariant};
use std::pin::Pin;

use super::qobject;

/// Qt's `UserRole`, the first role a model may define.
const USER_ROLE: i32 = 0x0100;

#[derive(Default)]
pub struct ListModelRust {
    pub roles: &'static [&'static str],
    pub(super) count: i32,
    rows: Vec<Vec<QVariant>>,
}

/// How `old` turns into `new`: rows `head..head + changed` change in place, then `removed` rows go or `inserted`
/// rows come at `head + changed`. Rows equal at both ends stay untouched.
#[derive(Debug, PartialEq, Eq)]
struct Splice {
    head: usize,
    changed: usize,
    removed: usize,
    inserted: usize,
}

fn splice<T: PartialEq>(old: &[T], new: &[T]) -> Splice {
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let tail = old[head..].iter().rev().zip(new[head..].iter().rev()).take_while(|(a, b)| a == b).count();
    let (old_middle, new_middle) = (old.len() - head - tail, new.len() - head - tail);
    let changed = old_middle.min(new_middle);
    Splice { head, changed, removed: old_middle - changed, inserted: new_middle - changed }
}

fn row(index: usize) -> i32 {
    i32::try_from(index).unwrap_or(i32::MAX)
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

    /// Shows `rows` with the fewest row changes, so a view keeps its delegates and its scroll position.
    pub fn replace(mut self: Pin<&mut Self>, rows: Vec<Vec<QVariant>>) {
        let Splice { head, changed, removed, inserted } = splice(&self.rows, &rows);
        let first = head + changed;
        let root = QModelIndex::default();
        if removed > 0 {
            self.as_mut().begin_remove_rows(&root, row(first), row(first + removed - 1));
            self.as_mut().rust_mut().rows = rows;
            self.as_mut().end_remove_rows();
        } else if inserted > 0 {
            self.as_mut().begin_insert_rows(&root, row(first), row(first + inserted - 1));
            self.as_mut().rust_mut().rows = rows;
            self.as_mut().end_insert_rows();
        } else {
            self.as_mut().rust_mut().rows = rows;
        }
        if changed > 0 {
            let top = self.index(row(head), 0, &root);
            let bottom = self.index(row(first - 1), 0, &root);
            self.as_mut().data_changed(&top, &bottom, &QList::default());
        }
        let count = row(self.rows.len());
        if self.count != count {
            self.as_mut().rust_mut().count = count;
            self.count_changed();
        }
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

#[cfg(test)]
mod tests {
    use super::{Splice, splice};

    #[test]
    fn a_new_age_text_changes_rows_in_place() {
        let old = [["a", "1 min"], ["b", "5 min"]];
        let new = [["a", "2 min"], ["b", "6 min"]];
        assert_eq!(splice(&old, &new), Splice { head: 0, changed: 2, removed: 0, inserted: 0 });
    }

    #[test]
    fn equal_rows_change_nothing() {
        let rows = [["a", "1 min"], ["b", "5 min"]];
        assert_eq!(splice(&rows, &rows), Splice { head: 2, changed: 0, removed: 0, inserted: 0 });
    }

    #[test]
    fn a_new_first_row_is_one_insert() {
        assert_eq!(splice(&["b", "c"], &["a", "b", "c"]), Splice { head: 0, changed: 0, removed: 0, inserted: 1 });
    }

    #[test]
    fn a_gone_row_is_one_remove() {
        assert_eq!(splice(&["a", "b", "c"], &["a", "c"]), Splice { head: 1, changed: 0, removed: 1, inserted: 0 });
    }
}
