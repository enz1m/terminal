// This renderer-neutral tree remains the durable session format, while
// `App::content` projects it into the framework's `enz_pane_grid`. The app owns
// pane identity/persistence; the framework owns recursive layout, divider
// interaction, theme ink, focus rings, and splitter accessibility.
//! Pane-layout tree for tmux-style side-by-side split panes.
//!
//! A `Session` owns a `Vec<Pane>` as leaf storage plus a `PaneLayout`
//! tree that describes HOW those panes tile inside the content area.
//! The tree is intentionally kept as a pure data structure operated
//! on by pure functions in this module — no PTY, no IO, no framework
//! types — so the split/close/focus/resize semantics are trivially
//! unit-testable and never entangled with the rendering path.
//!
//! Design:
//!
//! * `PaneLayout::Leaf(pane_id)` — one pane occupies the whole cell.
//! * `PaneLayout::Split { direction, ratio, first, second }` —
//!   two children stacked on the given axis. `ratio` is the LEADING
//!   child's share of the split (`first` = leading, `second` =
//!   trailing) as a fraction in `[MIN_RATIO, MAX_RATIO]`. The
//!   trailing child fills the rest.
//!
//! `SplitDirection::Horizontal` means "the SPLIT LINE is vertical,
//! panes are side-by-side left|right" — matching tmux `split-window
//! -h`. `SplitDirection::Vertical` means "the split line is
//! horizontal, panes are stacked top|bottom" — matching tmux
//! `split-window -v`.
//!
//! `flatten_to_rects` walks the tree and emits one `PaneRect` per
//! leaf, each with its `[0.0, 1.0]` fractional origin + size relative
//! to the containing pane cell. The renderer multiplies these
//! fractions by the content-pane's live pixel dimensions to place
//! absolute-positioned `TerminalGrid`s.

#[cfg(test)] // Only `prune_to` uses it, and that is test-only.
use std::collections::HashSet;

/// Minimum ratio for a split. Clamps prevent a pane from being resized down to
/// zero (which would make it un-focus-able). A dimensionless fraction of the
/// parent cell, not a pixel length.
///
/// Documented exception to the token rule. `SizingScale` measures widths and
/// heights, but the framework does have this role:
/// `enzui_widgets::pane_grid` publishes
/// [`MIN_RATIO`](enzui_widgets::pane_grid) = 0.05, the "smallest fraction
/// of its parent a pane may occupy". The app's floor is larger, and the
/// analogue is named here rather than denied.
///
/// The app adopts the framework pane grid with this stricter domain floor via
/// [`PaneLayout::pane_grid_state`]. A terminal pane is useful only if it holds
/// readable columns: at 520 px the generic 0.05 allows 26 px (under four
/// default cells), while 0.10 preserves approximately 52 px.
pub(crate) const MIN_RATIO: f32 = 0.10;

/// Maximum ratio for a split — **derived from [`MIN_RATIO`], not written
/// twice**.
///
/// The two constants are not independent: the maximum share of the leading
/// child is exactly the complement of the minimum share left for the trailing
/// one, or one side of a split gets a floor the other does not. The framework
/// derives its own band the same way, `OrderedRangeF32::new(MIN_RATIO, 1.0 -
/// MIN_RATIO)` in `pane_grid.rs`.
///
/// Guarded by `the_ratio_band_is_symmetric_and_derived`.
pub(crate) const MAX_RATIO: f32 = 1.0 - MIN_RATIO;

/// An even split — the ratio a freshly-created split starts at.
///
/// Documented exception to the token rule. 0.5 is not a tunable dimension but
/// the definition of "split this pane in half", which is what every terminal
/// multiplexer does on `split-window`. There is no framework analogue and there
/// should not be one: a token that could retune "half" off 0.5 would make a
/// split asymmetric for no reason the user asked for.
pub(crate) const DEFAULT_RATIO: f32 = 0.5;

/// Maximum terminal surfaces rendered in one session.
///
/// Each leaf is a complete terminal grid, scrollback chrome, pane header, and
/// accessibility subtree. Eight keeps the worst supported frame bounded while
/// still covering practical 2×2 and nested workspaces. A ninth split is
/// rejected by the reducer with visible feedback instead of creating hidden or
/// zero-sized work.
pub(crate) const MAX_PANES: usize = 8;

/// Step size for a keyboard resize nudge, as a ratio delta.
///
/// Documented exception to the token rule, as for [`MIN_RATIO`]. The framework
/// has this role: `pane_grid::KEYBOARD_STEP` = 0.02, "arrow-key resize step, as
/// a fraction of the split's extent". The app overrides that through
/// [`PaneLayout::pane_grid_state`]: terminal resizing is quantised to whole
/// columns, and 5% moves visibly instead of producing an apparently dead key.
pub(crate) const RESIZE_STEP: f32 = 0.05;

/// Theme-aware form used by the running application.
pub(crate) fn is_narrow_layout_with(
    content_pane_width_px: f32,
    pane_count: usize,
    sizing: &enzui_theme::SizingTokens,
) -> bool {
    content_pane_width_px < sizing.layout_switch_narrow && pane_count > 1
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SplitDirection {
    /// Split line vertical, children arranged LEFT | RIGHT.
    Horizontal,
    /// Split line horizontal, children arranged TOP | BOTTOM.
    Vertical,
}

/// The tree that describes how the current session's panes tile.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaneLayout {
    Leaf(String),
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: Box<PaneLayout>,
        second: Box<PaneLayout>,
    },
}

impl PaneLayout {
    /// Project the durable app tree into the framework widget's app-owned tree.
    #[must_use]
    pub(crate) fn pane_grid_tree(&self) -> enzui_widgets::PaneTree<String> {
        use enzui_widgets::{PaneSplitOrientation, PaneTree};
        match self {
            Self::Leaf(id) => PaneTree::pane(id.clone()),
            Self::Split {
                direction,
                ratio,
                first,
                second,
            } => PaneTree::split(
                match direction {
                    SplitDirection::Horizontal => PaneSplitOrientation::Vertical,
                    SplitDirection::Vertical => PaneSplitOrientation::Horizontal,
                },
                *ratio,
                first.pane_grid_tree(),
                second.pane_grid_tree(),
            ),
        }
    }

    /// Build framework renderer state with this terminal model's ratio band
    /// and keyboard step.
    #[must_use]
    pub(crate) fn pane_grid_state(&self) -> enzui_widgets::EnzPaneGridState<String> {
        let tokens = enzui_widgets::pane_grid::PaneGridTokens {
            min_ratio: MIN_RATIO,
            keyboard_step: RESIZE_STEP,
            ..Default::default()
        };
        enzui_widgets::EnzPaneGridState::with_tokens(self.pane_grid_tree(), tokens)
    }

    /// Copy framework divider ratios back into this persisted tree in the
    /// framework's documented depth-first splitter order.
    pub(crate) fn apply_pane_grid_ratios(&mut self, ratios: &[f32]) {
        fn apply(node: &mut PaneLayout, ratios: &[f32], next: &mut usize) {
            if let PaneLayout::Split {
                ratio,
                first,
                second,
                ..
            } = node
            {
                if let Some(value) = ratios.get(*next) {
                    *ratio = value.clamp(MIN_RATIO, MAX_RATIO);
                }
                *next += 1;
                apply(first, ratios, next);
                apply(second, ratios, next);
            }
        }
        apply(self, ratios, &mut 0);
    }

    /// A single-pane layout — the initial state when a session is
    /// created before the user has requested any splits.
    pub(crate) fn leaf(pane_id: impl Into<String>) -> Self {
        PaneLayout::Leaf(pane_id.into())
    }

    /// Number of leaves in this subtree. Cheap since sessions won't
    /// have huge splits.
    pub(crate) fn leaf_count(&self) -> usize {
        match self {
            PaneLayout::Leaf(_) => 1,
            PaneLayout::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    /// Collect every leaf's pane id in tree order (leftmost-first,
    /// then topmost-first depending on split axes). Used by focus-
    /// move and by tests.
    // No longer test-only — `pane_group::group_into`
    // records exactly which panes a merged session contributed, in its own
    // layout order, so ungrouping can give back the same arrangement.
    pub(crate) fn leaves_in_order(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    // Reachable from production through `leaves_in_order`.
    fn collect_leaves(&self, out: &mut Vec<String>) {
        match self {
            PaneLayout::Leaf(id) => out.push(id.clone()),
            PaneLayout::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    /// True if this pane id appears as a leaf anywhere.
    #[cfg(test)] // Reserved for the tiled pane renderer; see the module head.
    pub(crate) fn contains(&self, pane_id: &str) -> bool {
        match self {
            PaneLayout::Leaf(id) => id == pane_id,
            PaneLayout::Split { first, second, .. } => {
                first.contains(pane_id) || second.contains(pane_id)
            }
        }
    }

    /// Split the leaf identified by `target` into two: the original
    /// pane stays as `first`, `new_pane_id` becomes `second`. Returns
    /// `true` if the split happened. If `target` isn't a leaf under
    /// this subtree, no change is made.
    ///
    /// If the same tree is asked to split a `target` that already
    /// appears inside a `Split`, only the FIRST occurrence (in tree
    /// order) is split — leaves are unique by construction in this
    /// app, but the guarantee is written to keep the function total.
    #[must_use]
    pub(crate) fn split_leaf(
        &mut self,
        target: &str,
        direction: SplitDirection,
        new_pane_id: impl Into<String>,
    ) -> bool {
        let new_pane_id = new_pane_id.into();
        match self {
            PaneLayout::Leaf(id) if id == target => {
                let first = Box::new(PaneLayout::Leaf(id.clone()));
                let second = Box::new(PaneLayout::Leaf(new_pane_id));
                *self = PaneLayout::Split {
                    direction,
                    ratio: DEFAULT_RATIO,
                    first,
                    second,
                };
                true
            }
            PaneLayout::Leaf(_) => false,
            PaneLayout::Split { first, second, .. } => {
                first.split_leaf(target, direction, new_pane_id.clone())
                    || second.split_leaf(target, direction, new_pane_id)
            }
        }
    }

    /// Wrap the **whole** tree in a new split, putting `incoming` on the
    /// leading (`place_first`) or trailing side.
    ///
    /// # Why this is not `split_leaf`
    ///
    /// [`split_leaf`](Self::split_leaf) splits **one pane**, and always places
    /// the newcomer second. That is the right shape for `Split pane right`,
    /// which acts on the focused pane. It is the wrong shape for a session
    /// dropped on an **edge of the content area**: the user aimed at the
    /// content area, not at whichever pane happens to be focused inside it, so
    /// the incoming terminal takes half of the *whole* layout. Dropping on the
    /// left of a content area that is already split three ways must produce
    /// `newcomer | (the existing three)`, not a fourth pane wedged beside the
    /// focused one.
    ///
    /// `place_first` is the other half `split_leaf` cannot express: `Left` and
    /// `Up` put the newcomer on the leading side, `Right` and `Down` on the
    /// trailing side. Without it, all four edges would produce the same layout
    /// and two of the four drop targets would be lies.
    pub(crate) fn split_root(
        &mut self,
        direction: SplitDirection,
        incoming: PaneLayout,
        place_first: bool,
    ) {
        let existing = Box::new(std::mem::replace(self, PaneLayout::Leaf(String::new())));
        let incoming = Box::new(incoming);
        let (first, second) = if place_first {
            (incoming, existing)
        } else {
            (existing, incoming)
        };
        *self = PaneLayout::Split {
            direction,
            ratio: DEFAULT_RATIO,
            first,
            second,
        };
    }

    /// Split the leaf `target` around `incoming`, which may itself be a split
    /// tree, putting it on the leading (`place_first`) or trailing side: the
    /// shape of a session dropped onto one edge of one pane. Returns `false`,
    /// changing nothing, when `target` is not a leaf of this tree.
    #[must_use]
    pub(crate) fn split_leaf_with(
        &mut self,
        target: &str,
        direction: SplitDirection,
        incoming: PaneLayout,
        place_first: bool,
    ) -> bool {
        match self {
            PaneLayout::Leaf(id) if id == target => {
                self.split_root(direction, incoming, place_first);
                true
            }
            PaneLayout::Leaf(_) => false,
            PaneLayout::Split { first, second, .. } => {
                if first.leaves_in_order().iter().any(|id| id == target) {
                    first.split_leaf_with(target, direction, incoming, place_first)
                } else {
                    second.split_leaf_with(target, direction, incoming, place_first)
                }
            }
        }
    }

    /// Remove the leaf identified by `target`. When removing a leaf
    /// leaves the parent `Split` with only one remaining child, that
    /// child replaces the parent — the tree stays "no `Split` with
    /// fewer than two leaves". Returns `true` on removal.
    ///
    /// If `target` IS the last remaining leaf in the tree (i.e. the
    /// root is `Leaf(target)`), the function returns `false` — the
    /// caller should handle "close session" at a higher level rather
    /// than leaving the tree empty.
    #[must_use]
    pub(crate) fn remove_leaf(&mut self, target: &str) -> bool {
        // Cannot collapse into the root; the caller handles that case.
        if matches!(self, PaneLayout::Leaf(id) if id == target) {
            return false;
        }
        Self::remove_leaf_inner(self, target)
    }

    fn remove_leaf_inner(node: &mut PaneLayout, target: &str) -> bool {
        // Case 1: this node is a Split whose direct child is the target
        // leaf. Replace `node` with the sibling.
        if let PaneLayout::Split { first, second, .. } = node {
            let first_matches = matches!(first.as_ref(), PaneLayout::Leaf(id) if id == target);
            let second_matches = matches!(second.as_ref(), PaneLayout::Leaf(id) if id == target);
            if first_matches {
                let keep = std::mem::replace(second.as_mut(), PaneLayout::Leaf(String::new()));
                *node = keep;
                return true;
            }
            if second_matches {
                let keep = std::mem::replace(first.as_mut(), PaneLayout::Leaf(String::new()));
                *node = keep;
                return true;
            }
        }
        // Case 2: recurse; on child change, no restructuring needed
        // here because the recursive call already collapsed any
        // orphaned Split.
        match node {
            PaneLayout::Split { first, second, .. } => {
                Self::remove_leaf_inner(first, target) || Self::remove_leaf_inner(second, target)
            }
            PaneLayout::Leaf(_) => false,
        }
    }

    /// Nudge the ratio of the split whose subtree contains `pane_id`
    /// as its LEADING descendant. `delta` is added to the current
    /// ratio and clamped to `[MIN_RATIO, MAX_RATIO]`. Returns `true`
    /// if a resize was applied.
    ///
    /// This targets the NEAREST ancestor split of `pane_id`, so
    /// nudging works from inside a nested split — pressing "grow"
    /// on the current pane makes it larger against its sibling.
    #[must_use]
    #[cfg(test)] // Reserved for the tiled pane renderer; see the module head.
    pub(crate) fn resize_toward(&mut self, pane_id: &str, delta: f32) -> bool {
        // The parent-split resize applies to the split whose child
        // contains `pane_id`. If `pane_id` is in the LEADING subtree
        // we add `delta`; if it's in the TRAILING subtree we subtract
        // (so "grow current pane" is intuitive from either side).
        match self {
            PaneLayout::Leaf(_) => false,
            PaneLayout::Split {
                ratio,
                first,
                second,
                ..
            } => {
                // Prefer to recurse first so the DEEPEST parent-split
                // is the one resized. Only if neither child touches
                // its own descendant do we resize AT this node.
                if first.contains(pane_id) && first.resize_toward(pane_id, delta) {
                    return true;
                }
                if second.contains(pane_id) && second.resize_toward(pane_id, delta) {
                    return true;
                }
                // Now we're at the shallowest split touching `pane_id`.
                let signed = if first.contains(pane_id) {
                    delta
                } else if second.contains(pane_id) {
                    -delta
                } else {
                    return false;
                };
                let new_ratio = (*ratio + signed).clamp(MIN_RATIO, MAX_RATIO);
                if (new_ratio - *ratio).abs() < f32::EPSILON {
                    return false;
                }
                *ratio = new_ratio;
                true
            }
        }
    }

    /// Set the ratio of the split with `pane_id` in its LEADING
    /// subtree. Used when a UI drag handle reports an absolute
    /// value (e.g. divider dragged to 0.42).
    #[must_use]
    #[cfg(test)] // Reserved for the tiled pane renderer; see the module head.
    pub(crate) fn set_ratio_at(&mut self, pane_id: &str, new_ratio: f32) -> bool {
        match self {
            PaneLayout::Leaf(_) => false,
            PaneLayout::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if first.contains(pane_id) && first.set_ratio_at(pane_id, new_ratio) {
                    return true;
                }
                if second.contains(pane_id) && second.set_ratio_at(pane_id, new_ratio) {
                    return true;
                }
                if first.contains(pane_id) || second.contains(pane_id) {
                    let clamped = new_ratio.clamp(MIN_RATIO, MAX_RATIO);
                    if (clamped - *ratio).abs() < f32::EPSILON {
                        return false;
                    }
                    *ratio = clamped;
                    return true;
                }
                false
            }
        }
    }

    /// Prune any leaves whose id is not in `known`. Collapses splits
    /// whose only surviving child bubbles up to replace it. Returns
    /// the surviving pane ids in tree order. If the whole subtree is
    /// pruned, the tree collapses to `PaneLayout::Leaf(String::new
    /// ())` — the caller replaces this with a fresh leaf.
    #[cfg(test)] // Reserved for the tiled pane renderer; see the module head.
    pub(crate) fn prune_to(&mut self, known: &HashSet<String>) {
        loop {
            // Repeatedly walk the tree until no more collapses happen.
            let changed = Self::prune_step(self, known);
            if !changed {
                break;
            }
        }
    }

    #[cfg(test)] // Reserved for the tiled pane renderer; see the module head.
    fn prune_step(node: &mut PaneLayout, known: &HashSet<String>) -> bool {
        match node {
            PaneLayout::Leaf(_) => false,
            PaneLayout::Split { first, second, .. } => {
                // First recursively drop unknown leaves from children.
                let f_changed = Self::prune_step(first, known);
                let s_changed = Self::prune_step(second, known);
                // If EITHER child is a leaf pointing at an unknown id,
                // collapse `node` to the OTHER child.
                let f_bad = matches!(first.as_ref(), PaneLayout::Leaf(id) if !known.contains(id));
                let s_bad = matches!(second.as_ref(), PaneLayout::Leaf(id) if !known.contains(id));
                if f_bad && s_bad {
                    // Both children unknown — collapse to an empty leaf so
                    // the caller notices and rebuilds.
                    *node = PaneLayout::Leaf(String::new());
                    return true;
                }
                if f_bad {
                    let keep = std::mem::replace(second.as_mut(), PaneLayout::Leaf(String::new()));
                    *node = keep;
                    return true;
                }
                if s_bad {
                    let keep = std::mem::replace(first.as_mut(), PaneLayout::Leaf(String::new()));
                    *node = keep;
                    return true;
                }
                f_changed || s_changed
            }
        }
    }
}

/// One leaf's projected rectangle inside the containing content pane.
/// All fields are FRACTIONS in `[0.0, 1.0]` so the renderer can
/// multiply by live pixel dimensions without knowing the tree shape.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PaneRect {
    pub pane_id: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Walk a `PaneLayout` and produce one `PaneRect` per leaf. Given
/// `(0, 0, 1, 1)` the output covers the whole content pane.
pub(crate) fn flatten_to_rects(layout: &PaneLayout) -> Vec<PaneRect> {
    let mut out = Vec::with_capacity(layout.leaf_count());
    flatten_recursive(layout, 0.0, 0.0, 1.0, 1.0, &mut out);
    out
}

fn flatten_recursive(node: &PaneLayout, x: f32, y: f32, w: f32, h: f32, out: &mut Vec<PaneRect>) {
    match node {
        PaneLayout::Leaf(id) => out.push(PaneRect {
            pane_id: id.clone(),
            x,
            y,
            w,
            h,
        }),
        PaneLayout::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let r = ratio.clamp(MIN_RATIO, MAX_RATIO);
            match direction {
                SplitDirection::Horizontal => {
                    let first_w = w * r;
                    flatten_recursive(first, x, y, first_w, h, out);
                    flatten_recursive(second, x + first_w, y, w - first_w, h, out);
                }
                SplitDirection::Vertical => {
                    let first_h = h * r;
                    flatten_recursive(first, x, y, w, first_h, out);
                    flatten_recursive(second, x, y + first_h, w, h - first_h, out);
                }
            }
        }
    }
}

/// Directional focus move: given the current focus, find the leaf
/// whose rect is closest along `direction`. If nothing lies in that
/// direction (e.g. pressing Right from the rightmost pane), the
/// current pane keeps focus. Uses the geometric flatten to make the
/// decision so the ordering matches what the user SEES on screen.
pub(crate) fn focus_direction(layout: &PaneLayout, current: &str, dir: FocusDir) -> Option<String> {
    let rects = flatten_to_rects(layout);
    let cur = rects.iter().find(|r| r.pane_id == current)?;
    // The candidate anchor: for Right, we look at each pane's LEFT
    // edge and pick the one just past the current pane's RIGHT edge.
    // Same logic mirrored for the other three directions.
    let mut best: Option<(&PaneRect, f32)> = None;
    for candidate in &rects {
        if candidate.pane_id == current {
            continue;
        }
        let (aligned, distance) = match dir {
            FocusDir::Right => {
                if candidate.x + 1e-4 < cur.x + cur.w {
                    continue;
                }
                // Prefer panes overlapping vertically with current.
                let overlap = vertical_overlap(cur, candidate);
                if overlap <= 0.0 {
                    continue;
                }
                (true, candidate.x - (cur.x + cur.w))
            }
            FocusDir::Left => {
                if candidate.x + candidate.w > cur.x + 1e-4 {
                    continue;
                }
                let overlap = vertical_overlap(cur, candidate);
                if overlap <= 0.0 {
                    continue;
                }
                (true, cur.x - (candidate.x + candidate.w))
            }
            FocusDir::Down => {
                if candidate.y + 1e-4 < cur.y + cur.h {
                    continue;
                }
                let overlap = horizontal_overlap(cur, candidate);
                if overlap <= 0.0 {
                    continue;
                }
                (true, candidate.y - (cur.y + cur.h))
            }
            FocusDir::Up => {
                if candidate.y + candidate.h > cur.y + 1e-4 {
                    continue;
                }
                let overlap = horizontal_overlap(cur, candidate);
                if overlap <= 0.0 {
                    continue;
                }
                (true, cur.y - (candidate.y + candidate.h))
            }
        };
        if !aligned {
            continue;
        }
        if best.is_none_or(|(_, d)| distance < d) {
            best = Some((candidate, distance));
        }
    }
    best.map(|(r, _)| r.pane_id.clone())
}

fn vertical_overlap(a: &PaneRect, b: &PaneRect) -> f32 {
    let top = a.y.max(b.y);
    let bottom = (a.y + a.h).min(b.y + b.h);
    (bottom - top).max(0.0)
}

fn horizontal_overlap(a: &PaneRect, b: &PaneRect) -> f32 {
    let left = a.x.max(b.x);
    let right = (a.x + a.w).min(b.x + b.w);
    (right - left).max(0.0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FocusDir {
    Left,
    Right,
    Up,
    Down,
}
