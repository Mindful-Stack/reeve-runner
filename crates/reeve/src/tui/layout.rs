//! Layout-mode dispatch. Adding a new layout = new variant + render fn.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    /// All 6 columns visible across the full width.
    KanbanFull,
    /// Focused column on the left (~35%), detail pane on the right (~65%).
    FocusedColumnPlusDetail,
}

impl Default for LayoutMode {
    fn default() -> Self {
        Self::KanbanFull
    }
}
