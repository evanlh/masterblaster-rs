//! Undo/redo stack for pattern editing.

use mb_ir::Edit;

/// A single undoable operation: forward edit + reverse edit.
#[derive(Clone, Debug)]
struct UndoEntry {
    forward: Vec<Edit>,
    reverse: Vec<Edit>,
}

/// Undo/redo stack.
pub struct UndoStack {
    entries: Vec<UndoEntry>,
    position: usize,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            position: 0,
        }
    }

    /// Record a single edit with its reverse.
    pub fn push(&mut self, forward: Edit, reverse: Edit) {
        self.push_batch(vec![forward], vec![reverse]);
    }

    /// Record a batch of edits (e.g., paste) as a single undo entry.
    pub fn push_batch(&mut self, forward: Vec<Edit>, reverse: Vec<Edit>) {
        // Truncate any redo history beyond current position
        self.entries.truncate(self.position);
        self.entries.push(UndoEntry { forward, reverse });
        self.position = self.entries.len();
    }

    /// Undo: returns the reverse edits to apply, or None if nothing to undo.
    pub fn undo(&mut self) -> Option<&[Edit]> {
        if self.position == 0 {
            return None;
        }
        self.position -= 1;
        Some(&self.entries[self.position].reverse)
    }

    /// Redo: returns the forward edits to apply, or None if nothing to redo.
    pub fn redo(&mut self) -> Option<&[Edit]> {
        if self.position >= self.entries.len() {
            return None;
        }
        let edits = &self.entries[self.position].forward;
        self.position += 1;
        Some(edits)
    }

    #[allow(dead_code)]
    pub fn can_undo(&self) -> bool {
        self.position > 0
    }

    #[allow(dead_code)]
    pub fn can_redo(&self) -> bool {
        self.position < self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_ir::{Cell, Note};

    fn set_cell(row: u16, note: Note) -> Edit {
        Edit::SetCell {
            pattern: 0,
            row,
            channel: 0,
            cell: Cell { note, ..Cell::empty() },
        }
    }

    #[test]
    fn undo_redo_single() {
        let mut stack = UndoStack::new();
        let fwd = set_cell(0, Note::On(60));
        let rev = set_cell(0, Note::None);
        stack.push(fwd, rev.clone());

        assert!(stack.can_undo());
        let undone = stack.undo().unwrap();
        assert_eq!(undone.len(), 1);
        assert_eq!(undone[0], rev);

        assert!(stack.can_redo());
    }

    #[test]
    fn undo_at_bottom_returns_none() {
        let mut stack = UndoStack::new();
        assert!(stack.undo().is_none());
    }

    #[test]
    fn redo_at_top_returns_none() {
        let mut stack = UndoStack::new();
        assert!(stack.redo().is_none());
    }

    #[test]
    fn new_edit_after_undo_truncates_redo() {
        let mut stack = UndoStack::new();
        stack.push(set_cell(0, Note::On(60)), set_cell(0, Note::None));
        stack.push(set_cell(1, Note::On(62)), set_cell(1, Note::None));

        stack.undo(); // undo second edit
        assert!(stack.can_redo());

        // New edit truncates redo history
        stack.push(set_cell(2, Note::On(64)), set_cell(2, Note::None));
        assert!(!stack.can_redo());
    }

    #[test]
    fn batch_undo() {
        let mut stack = UndoStack::new();
        let fwd = vec![set_cell(0, Note::On(60)), set_cell(1, Note::On(62))];
        let rev = vec![set_cell(0, Note::None), set_cell(1, Note::None)];
        stack.push_batch(fwd, rev);

        let undone = stack.undo().unwrap();
        assert_eq!(undone.len(), 2);
    }
}
