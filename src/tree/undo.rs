//! Undo/redo stack for tree operations
//!
//! Provides a simple undo/redo mechanism for tree manipulation operations.
//! Currently a stub implementation to be expanded in Phase 4.

use super::node::NodeId;
use crate::publication::NAddr;

/// Maximum number of operations to keep in history
const MAX_UNDO_HISTORY: usize = 100;

/// The undo/redo stack
#[derive(Debug, Clone, Default)]
pub struct UndoStack {
    /// Past operations that can be undone
    undo: Vec<Operation>,
    /// Undone operations that can be redone
    redo: Vec<Operation>,
}

impl UndoStack {
    /// Create a new empty undo stack
    pub fn new() -> Self {
        UndoStack {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Push an operation onto the undo stack
    pub fn push(&mut self, op: Operation) {
        // Clear redo stack when new operation is performed
        self.redo.clear();

        self.undo.push(op);

        // Trim history if too long
        while self.undo.len() > MAX_UNDO_HISTORY {
            self.undo.remove(0);
        }
    }

    /// Pop the last operation for undo
    pub fn pop_undo(&mut self) -> Option<Operation> {
        self.undo.pop()
    }

    /// Pop the last undone operation for redo
    pub fn pop_redo(&mut self) -> Option<Operation> {
        self.redo.pop()
    }

    /// Move an operation to the redo stack (called after undo)
    pub fn push_redo(&mut self, op: Operation) {
        self.redo.push(op);
    }

    /// Check if undo is available
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Check if redo is available
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Clear all history
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// Get the description of the next undo operation
    pub fn undo_description(&self) -> Option<&str> {
        self.undo.last().map(|op| op.description())
    }

    /// Get the description of the next redo operation
    pub fn redo_description(&self) -> Option<&str> {
        self.redo.last().map(|op| op.description())
    }
}

/// An undoable operation
#[derive(Debug, Clone)]
pub enum Operation {
    /// Section was moved within its parent
    MoveSection {
        node_id: NodeId,
        parent_id: NodeId,
        old_position: usize,
        new_position: usize,
    },
    /// Section was deleted
    DeleteSection {
        node_id: NodeId,
        parent_id: NodeId,
        position: usize,
        addr: NAddr,
        title: Option<String>,
        content: Option<String>,
    },
    /// Section was pasted/inserted
    InsertSection {
        node_id: NodeId,
        parent_id: NodeId,
        position: usize,
    },
    /// Version was slotted in
    SlotVersion {
        node_id: NodeId,
        old_version_index: usize,
        new_version_index: usize,
    },
    /// Multiple operations grouped together
    Batch {
        operations: Vec<Operation>,
        description: String,
    },
}

impl Operation {
    /// Get a human-readable description of this operation
    pub fn description(&self) -> &str {
        match self {
            Operation::MoveSection { .. } => "Move section",
            Operation::DeleteSection { .. } => "Delete section",
            Operation::InsertSection { .. } => "Insert section",
            Operation::SlotVersion { .. } => "Change version",
            Operation::Batch { description, .. } => description,
        }
    }

    /// Get the inverse operation (for undo)
    pub fn inverse(&self) -> Operation {
        match self {
            Operation::MoveSection {
                node_id,
                parent_id,
                old_position,
                new_position,
            } => Operation::MoveSection {
                node_id: *node_id,
                parent_id: *parent_id,
                old_position: *new_position,
                new_position: *old_position,
            },
            Operation::InsertSection {
                node_id,
                parent_id,
                position,
            } => {
                // Note: For a proper inverse, we'd need the original data
                // This is a simplified version - placeholder for Phase 4
                Operation::DeleteSection {
                    node_id: *node_id,
                    parent_id: *parent_id,
                    position: *position,
                    addr: NAddr::new(0, "", ""),
                    title: None,
                    content: None,
                }
            }
            Operation::DeleteSection {
                node_id,
                parent_id,
                position,
                addr: _,
                title: _,
                content: _,
            } => Operation::InsertSection {
                node_id: *node_id,
                parent_id: *parent_id,
                position: *position,
            },
            Operation::SlotVersion {
                node_id,
                old_version_index,
                new_version_index,
            } => Operation::SlotVersion {
                node_id: *node_id,
                old_version_index: *new_version_index,
                new_version_index: *old_version_index,
            },
            Operation::Batch {
                operations,
                description,
            } => Operation::Batch {
                operations: operations.iter().rev().map(|op| op.inverse()).collect(),
                description: format!("Undo: {}", description),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_undo_stack_basic() {
        let mut stack = UndoStack::new();

        assert!(!stack.can_undo());
        assert!(!stack.can_redo());

        let op = Operation::MoveSection {
            node_id: NodeId(1),
            parent_id: NodeId(0),
            old_position: 0,
            new_position: 1,
        };

        stack.push(op.clone());
        assert!(stack.can_undo());
        assert!(!stack.can_redo());

        let undone = stack.pop_undo().unwrap();
        stack.push_redo(undone);

        assert!(!stack.can_undo());
        assert!(stack.can_redo());
    }

    #[test]
    fn test_redo_cleared_on_new_operation() {
        let mut stack = UndoStack::new();

        let op1 = Operation::MoveSection {
            node_id: NodeId(1),
            parent_id: NodeId(0),
            old_position: 0,
            new_position: 1,
        };

        stack.push(op1);
        let undone = stack.pop_undo().unwrap();
        stack.push_redo(undone);

        assert!(stack.can_redo());

        // New operation should clear redo
        let op2 = Operation::MoveSection {
            node_id: NodeId(2),
            parent_id: NodeId(0),
            old_position: 1,
            new_position: 2,
        };
        stack.push(op2);

        assert!(!stack.can_redo());
    }
}
