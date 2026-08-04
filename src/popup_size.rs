use std::borrow::Cow;

use crate::layout::PaneId;
use ratatui::layout::Rect;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PopupSize {
    Cells(u16),
    Percent(u8),
}

impl PopupSize {
    pub(crate) fn resolve(self, available: u16) -> u16 {
        match self {
            Self::Cells(cells) => cells,
            Self::Percent(percent) => ((available as u32 * percent as u32) / 100) as u16,
        }
    }

    pub(crate) fn parse_cli(value: &str) -> Result<Self, String> {
        if let Some(percent) = value.strip_suffix('%') {
            let percent = percent
                .parse::<u8>()
                .map_err(|_| "must be a number of cells or a percentage like 80%".to_string())?;
            if !(1..=100).contains(&percent) {
                return Err("percentage must be between 1% and 100%".to_string());
            }
            return Ok(Self::Percent(percent));
        }
        value
            .parse::<u16>()
            .map(Self::Cells)
            .map_err(|_| "must be a number of cells or a percentage like 80%".to_string())
    }

    fn parse_percent_string(value: &str) -> Result<Self, String> {
        if value.ends_with('%') {
            return Self::parse_cli(value);
        }
        Err("string sizes must be percentages like 80%; use a number for cells".to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PopupResolvedGeometry {
    pub outer: Rect,
    pub inner: Rect,
}

pub(crate) fn resolve_popup_geometry(
    width: Option<PopupSize>,
    height: Option<PopupSize>,
    area: Rect,
) -> Option<PopupResolvedGeometry> {
    let default_width = area.width.saturating_div(2).max(6);
    let default_height = area.height.saturating_div(2).max(4);
    let outer_width = width
        .map(|width| width.resolve(area.width))
        .unwrap_or(default_width)
        .max(6)
        .min(area.width);
    let outer_height = height
        .map(|height| height.resolve(area.height))
        .unwrap_or(default_height)
        .max(4)
        .min(area.height);
    if outer_width < 6 || outer_height < 4 {
        return None;
    }

    let outer_x = area.x + (area.width.saturating_sub(outer_width)) / 2;
    let outer_y = area.y + (area.height.saturating_sub(outer_height)) / 2;
    let pane_inner_width = outer_width.saturating_sub(2);
    let pane_inner_height = outer_height.saturating_sub(2);
    let terminal_cols = if pane_inner_width <= 4 {
        pane_inner_width
    } else {
        pane_inner_width.saturating_sub(1)
    };
    let inner = Rect::new(
        outer_x.saturating_add(1),
        outer_y.saturating_add(1),
        terminal_cols,
        pane_inner_height,
    );
    Some(PopupResolvedGeometry {
        outer: Rect::new(outer_x, outer_y, outer_width, outer_height),
        inner,
    })
}

/// Cap on preview rows drawn above a floating pane's stack, regardless of
/// how many floats are hidden or how much vertical space is free.
const MAX_STACK_PREVIEW_ROWS: u16 = 8;

/// What a single stack preview row represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StackBarKind {
    /// A hidden float; clicking this row's rect brings it to the front.
    Pane(PaneId),
    /// Folds `count` further hidden floats that didn't fit as individual rows.
    Summary { count: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StackBar {
    pub rect: Rect,
    pub kind: StackBarKind,
}

/// One preview row per entry in `hidden`, stacked directly above
/// `popup_outer`, capped to the space between `area`'s top edge and the
/// popup and to `MAX_STACK_PREVIEW_ROWS`. `hidden` is ordered back-to-front
/// (oldest first); when not everything fits, the oldest floats are folded
/// into a single summary row at the top of the stack.
pub(crate) fn stack_bar_rects(hidden: &[PaneId], popup_outer: Rect, area: Rect) -> Vec<StackBar> {
    if hidden.is_empty() {
        return Vec::new();
    }
    let space_above = popup_outer.y.saturating_sub(area.y);
    let max_bars = space_above.min(MAX_STACK_PREVIEW_ROWS) as usize;
    if max_bars == 0 {
        return Vec::new();
    }

    let kinds: Vec<StackBarKind> = if hidden.len() <= max_bars {
        hidden.iter().map(|id| StackBarKind::Pane(*id)).collect()
    } else {
        let real_bars = max_bars - 1;
        let folded = hidden.len() - real_bars;
        let mut kinds = vec![StackBarKind::Summary { count: folded }];
        kinds.extend(
            hidden[hidden.len() - real_bars..]
                .iter()
                .map(|id| StackBarKind::Pane(*id)),
        );
        kinds
    };

    let total = kinds.len() as u16;
    kinds
        .into_iter()
        .enumerate()
        .map(|(i, kind)| StackBar {
            rect: Rect::new(
                popup_outer.x,
                popup_outer.y.saturating_sub(total - i as u16),
                popup_outer.width,
                1,
            ),
            kind,
        })
        .collect()
}

impl Serialize for PopupSize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Cells(cells) => serializer.serialize_u16(*cells),
            Self::Percent(percent) => serializer.serialize_str(&format!("{percent}%")),
        }
    }
}

impl<'de> Deserialize<'de> for PopupSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PopupSizeVisitor;

        impl serde::de::Visitor<'_> for PopupSizeVisitor {
            type Value = PopupSize;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a cell count or percentage string like 80%")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let value =
                    u16::try_from(value).map_err(|_| E::custom("cell count must fit in u16"))?;
                Ok(PopupSize::Cells(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let value = u16::try_from(value)
                    .map_err(|_| E::custom("cell count must be between 0 and 65535"))?;
                Ok(PopupSize::Cells(value))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                PopupSize::parse_percent_string(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_any(PopupSizeVisitor)
    }
}

impl schemars::JsonSchema for PopupSize {
    fn schema_name() -> Cow<'static, str> {
        "PopupSize".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "oneOf": [
                {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 65535,
                    "description": "Outer popup size in terminal cells, including the border."
                },
                {
                    "type": "string",
                    "pattern": "^(100|[1-9][0-9]?)%$",
                    "description": "Outer popup size as a percentage of the terminal area, for example 80%."
                }
            ]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::PopupSize;
    use super::{stack_bar_rects, StackBarKind, MAX_STACK_PREVIEW_ROWS};

    #[test]
    fn parses_cells_and_percent() {
        assert_eq!(PopupSize::parse_cli("120"), Ok(PopupSize::Cells(120)));
        assert_eq!(PopupSize::parse_cli("80%"), Ok(PopupSize::Percent(80)));
        assert_eq!(PopupSize::Percent(80).resolve(100), 80);
    }

    #[test]
    fn rejects_invalid_percent() {
        assert!(PopupSize::parse_cli("0%").is_err());
        assert!(PopupSize::parse_cli("101%").is_err());
        assert!(PopupSize::parse_cli("%").is_err());
    }

    #[test]
    fn string_deserialization_requires_percent() {
        assert!(serde_json::from_value::<PopupSize>(serde_json::json!("120")).is_err());
        assert_eq!(
            serde_json::from_value::<PopupSize>(serde_json::json!("80%")).unwrap(),
            PopupSize::Percent(80)
        );
    }

    #[test]
    fn serializes_percent_as_string() {
        assert_eq!(
            serde_json::to_value(PopupSize::Percent(80)).unwrap(),
            serde_json::json!("80%")
        );
        assert_eq!(
            serde_json::to_value(PopupSize::Cells(120)).unwrap(),
            serde_json::json!(120)
        );
    }

    #[test]
    fn resolves_requested_outer_size_and_inner_terminal_area() {
        let resolved = super::resolve_popup_geometry(
            Some(PopupSize::Percent(80)),
            Some(PopupSize::Percent(40)),
            ratatui::layout::Rect::new(0, 0, 100, 30),
        )
        .unwrap();
        assert_eq!(resolved.outer, ratatui::layout::Rect::new(10, 9, 80, 12));
        assert_eq!(resolved.inner, ratatui::layout::Rect::new(11, 10, 77, 10));
    }

    #[test]
    fn allows_full_terminal_outer_size() {
        let resolved = super::resolve_popup_geometry(
            Some(PopupSize::Percent(100)),
            Some(PopupSize::Percent(100)),
            ratatui::layout::Rect::new(4, 2, 100, 30),
        )
        .unwrap();

        assert_eq!(resolved.outer, ratatui::layout::Rect::new(4, 2, 100, 30));
        assert_eq!(resolved.inner, ratatui::layout::Rect::new(5, 3, 97, 28));
    }

    #[test]
    fn enforces_runtime_minimum_terminal_width() {
        let resolved = super::resolve_popup_geometry(
            Some(PopupSize::Cells(4)),
            None,
            ratatui::layout::Rect::new(0, 0, 80, 24),
        )
        .unwrap();
        assert_eq!(resolved.outer.width, 6);
        assert_eq!(resolved.inner.width, 4);

        assert!(
            super::resolve_popup_geometry(None, None, ratatui::layout::Rect::new(0, 0, 5, 24),)
                .is_none()
        );
    }

    #[test]
    fn stack_bar_rects_is_empty_when_no_hidden_floats() {
        let popup = super::Rect::new(5, 10, 20, 6);
        let area = super::Rect::new(0, 0, 80, 24);
        assert!(stack_bar_rects(&[], popup, area).is_empty());
    }

    #[test]
    fn stack_bar_rects_renders_one_bar_per_hidden_float_when_space_allows() {
        let hidden = [
            crate::layout::PaneId::from_raw(1),
            crate::layout::PaneId::from_raw(2),
        ];
        let popup = super::Rect::new(5, 10, 20, 6);
        let area = super::Rect::new(0, 0, 80, 24);
        let bars = stack_bar_rects(&hidden, popup, area);
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].rect, super::Rect::new(5, 8, 20, 1));
        assert!(matches!(bars[0].kind, StackBarKind::Pane(id) if id == hidden[0]));
        assert_eq!(bars[1].rect, super::Rect::new(5, 9, 20, 1));
        assert!(matches!(bars[1].kind, StackBarKind::Pane(id) if id == hidden[1]));
    }

    #[test]
    fn stack_bar_rects_folds_overflow_into_a_summary_row_when_space_is_tight() {
        let hidden: Vec<_> = (1..=5).map(crate::layout::PaneId::from_raw).collect();
        let popup = super::Rect::new(5, 2, 20, 6);
        let area = super::Rect::new(0, 0, 80, 24);
        let bars = stack_bar_rects(&hidden, popup, area);
        // space_above = popup.y(2) - area.y(0) = 2, so only 2 rows fit:
        // 1 summary row + 1 real bar for the most-recently-hidden float.
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].rect, super::Rect::new(5, 0, 20, 1));
        assert!(matches!(bars[0].kind, StackBarKind::Summary { count: 4 }));
        assert_eq!(bars[1].rect, super::Rect::new(5, 1, 20, 1));
        assert!(matches!(bars[1].kind, StackBarKind::Pane(id) if id == hidden[4]));
    }

    #[test]
    fn stack_bar_rects_caps_at_max_stack_preview_rows_even_with_room_to_spare() {
        let hidden: Vec<_> = (1..=20).map(crate::layout::PaneId::from_raw).collect();
        let popup = super::Rect::new(5, 15, 20, 6);
        let area = super::Rect::new(0, 0, 80, 24);
        let bars = stack_bar_rects(&hidden, popup, area);
        assert_eq!(bars.len(), MAX_STACK_PREVIEW_ROWS as usize);
        assert!(matches!(bars[0].kind, StackBarKind::Summary { count: 13 }));
        assert!(matches!(bars[1].kind, StackBarKind::Pane(id) if id == hidden[13]));
        assert!(matches!(bars[7].kind, StackBarKind::Pane(id) if id == hidden[19]));
    }

    #[test]
    fn stack_bar_rects_folds_everything_into_one_summary_row_when_only_one_row_fits() {
        let hidden: Vec<_> = (1..=4).map(crate::layout::PaneId::from_raw).collect();
        let popup = super::Rect::new(5, 1, 20, 6);
        let area = super::Rect::new(0, 0, 80, 24);
        let bars = stack_bar_rects(&hidden, popup, area);
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].rect, super::Rect::new(5, 0, 20, 1));
        assert!(matches!(bars[0].kind, StackBarKind::Summary { count: 4 }));
    }

    #[test]
    fn stack_bar_rects_is_empty_when_popup_touches_the_top_edge() {
        let hidden = [crate::layout::PaneId::from_raw(1)];
        let popup = super::Rect::new(5, 0, 20, 6);
        let area = super::Rect::new(0, 0, 80, 24);
        assert!(stack_bar_rects(&hidden, popup, area).is_empty());
    }
}
