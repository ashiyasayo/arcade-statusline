use crate::helpers::*;
use crate::models::{Input, RawState};
use crate::settings::read_effort_level;
use crate::state::{current_slot, slot_pos};
use crate::themes::Theme;

const FUTURE_SLOTS: u64 = 4;

const SMALL_FLOWERS: [&str; 3] = ["🌸", "🌺", "🌼"];
const BIG_FLOWERS: [&str; 3] = ["🪻", "🌻", "🌷"];
const FRUITS: [&str; 7] = ["🍎", "🍊", "🍋", "🍇", "🍓", "🍑", "🫐"];

const ANIM_PATH: &str = "/tmp/.claude-pikmin-anim";

fn pick_small(slot: u64) -> &'static str {
    SMALL_FLOWERS[(slot % 3) as usize]
}

fn pick_big(slot: u64) -> &'static str {
    BIG_FLOWERS[(slot % 3) as usize]
}

fn pick_fruit(slot: u64) -> &'static str {
    FRUITS[(slot % FRUITS.len() as u64) as usize]
}

/// Toggle animation state, return pikmin order string
fn pikmin_squad() -> &'static str {
    let current = std::fs::read_to_string(ANIM_PATH)
        .unwrap_or_default()
        .trim()
        .to_string();
    if current == "1" {
        let _ = std::fs::write(ANIM_PATH, "0");
        "🔴🟡🔵"
    } else {
        let _ = std::fs::write(ANIM_PATH, "1");
        "🟡🔵🔴"
    }
}

/// Render a single slot's display characters.
/// `narrow_emoji`: true when terminal renders emoji as 1 col (JetBrains) — add HS to compensate.
fn render_slot(val: &str, pos: u8, narrow_emoji: bool) -> String {
    let is_wide = pos == 0 || pos == 7;
    if val == "·" || val.is_empty() {
        if narrow_emoji {
            if is_wide {
                format!("{DIM}·{NC}{DIM}·{NC}")
            } else {
                format!("{DIM}·{NC}")
            }
        } else {
            format!("{DIM}·{NC} ") // 2 cols to match emoji width
        }
    } else if is_wide {
        if narrow_emoji { format!("{val}{HS}") } else { format!("{val}") }
    } else {
        val.to_string()
    }
}

/// Determine what to display for a trail slot
fn trail_content(slot: u64, state: &RawState) -> &'static str {
    let pos = slot_pos(slot);
    let key = slot.to_string();
    let consumed = state.slots.get(&key).is_some_and(|r| r.consumed);

    if consumed {
        if pos == 0 { pick_big(slot) } else { pick_small(slot) }
    } else if pos == 0 {
        "🌱"
    } else {
        "·"
    }
}

/// Determine what to display for a future slot
fn future_content(slot: u64, five_reset_slot: Option<u64>, week_reset_slot: Option<u64>) -> &'static str {
    let pos = slot_pos(slot);
    let is_fruit = five_reset_slot == Some(slot) || week_reset_slot == Some(slot);

    if is_fruit {
        pick_fruit(slot)
    } else if pos == 0 {
        "🌱"
    } else {
        "·"
    }
}

/// 將左右資訊對齊到固定寬度；內容放不下時交由呼叫端選擇較精簡的版本。
fn align_status_line(
    left: &str,
    right: &str,
    width: usize,
    narrow_emoji: bool,
) -> Option<String> {
    let content_width = visible_width_ex(left, narrow_emoji)
        + visible_width_ex(right, narrow_emoji);
    if content_width > width {
        return None;
    }
    Some(format!("{left}{}{right}", " ".repeat(width - content_width)))
}

pub struct Pikmin {
    pub narrow_emoji: bool,
    pub buddy_padding: bool,
}

impl Theme for Pikmin {
    fn render(&self, input: &Input, state: &RawState, max_width: usize) -> String {
        let ne = self.narrow_emoji;
        let slot = current_slot();
        let ctx_remain = input.context_window.remain();
        let five = &input.rate_limits.five_hour;
        let week = &input.rate_limits.seven_day;
        let rate_hit = five.as_ref().is_some_and(|r| r.is_hit())
            || week.as_ref().is_some_and(|r| r.is_hit());

        // Reset time slots (for fruit in future)
        let slot_secs = 15 * 60;
        let five_reset_slot = five.as_ref()
            .and_then(|r| r.resets_at)
            .map(|t| t / slot_secs);
        let week_reset_slot = week.as_ref()
            .and_then(|r| r.resets_at)
            .map(|t| t / slot_secs);

        // ── Game line (build first to determine map_w) ──────────────
        let pikmin = pikmin_squad();
        let mut game = String::new();

        // 先保留隊伍與未來四格，再依剩餘寬度加入最近的足跡，避免窄終端換行。
        let squad_spacer = if ne { HS } else { "" };
        let squad = format!("{pikmin}{squad_spacer}");

        // Future
        let mut future = String::new();
        for s in (slot + 1)..=(slot + FUTURE_SLOTS) {
            let content = future_content(s, five_reset_slot, week_reset_slot);
            let pos = slot_pos(s);
            future += &render_slot(content, pos, ne);
        }

        let fixed_width = visible_width_ex(&squad, ne) + visible_width_ex(&future, ne);
        let target_width = max_width.max(fixed_width);
        let mut trail_parts = Vec::new();
        let mut used_width = fixed_width;
        let mut offset = 0;
        while used_width < target_width {
            let trail_slot = slot.saturating_sub(offset);
            let part = render_slot(trail_content(trail_slot, state), slot_pos(trail_slot), ne);
            let part_width = visible_width_ex(&part, ne);
            if used_width + part_width > target_width {
                break;
            }
            used_width += part_width;
            trail_parts.push(part);
            offset += 1;
        }
        trail_parts.reverse();
        // 格子的顯示寬度可能是 1 或 2；剩餘一欄以空白補齊，確保整行等於 COLUMNS。
        game += &" ".repeat(target_width - used_width);
        game += &trail_parts.concat();
        game += &squad;
        game += &future;

        // Derive map_w from game line's actual visible width (terminal-aware)
        let map_w = visible_width_ex(&game, ne);

        // ── Header ──────────────────────────────────────────────────
        let next_pos = slot_pos(slot + 1);
        let next_flower = if next_pos == 0 {
            pick_big(slot + 1)
        } else {
            pick_small(slot + 1)
        };

        let model_str = input.model.display().unwrap_or_default();
        let ctx_size = input.context_window.size().map(fmt_tokens).unwrap_or_default();
        let left_base = format!("{ORANGE}CLAUDE{NC}");
        let mut left_full = left_base.clone();
        if let Some(v) = &input.version {
            left_full += &format!(" {DIM}v{v}{NC}");
        }

        let mut right_full = String::new();
        if !model_str.is_empty() {
            right_full += &format!("{DIM}{model_str}{NC} ");
        }
        if !ctx_size.is_empty() {
            right_full += &format!("{DIM}{ctx_size}{NC} ");
        }
        let ctx_c = colour_remain(ctx_remain);
        let flower_spacer = if ne { HS } else { "" };
        let right_context = format!(
            "{DIM}Context{NC} {next_flower}{flower_spacer}{ctx_c} {DIM}left{NC}"
        );
        right_full += &right_context;
        let right_compact = format!("{DIM}C{NC} {next_flower}{flower_spacer}{ctx_c}");

        let header = [
            (&left_full, &right_full),
            (&left_base, &right_full),
            (&left_base, &right_context),
            (&String::new(), &right_context),
            (&String::new(), &right_compact),
        ]
        .into_iter()
        .find_map(|(left, right)| align_status_line(left, right, map_w, ne))
        .unwrap_or_else(|| pad_to_width_ex(&right_compact, map_w, ne));

        // ── Sky ─────────────────────────────────────────────────────
        let sky_color = "\x1b[38;5;39m";
        let sky: String = (0..map_w).map(|_| format!("{sky_color}█{NC}")).collect();

        // ── Grass ───────────────────────────────────────────────────
        let grass_color = "\x1b[38;5;28m";
        let grass: String = (0..map_w).map(|_| format!("{grass_color}▀{NC}")).collect();

        // ── Footer ──────────────────────────────────────────────────
        let mut footer_left = String::new();
        let mut footer_left_compact = String::new();

        if let Some(w) = week {
            let w_remain = w.remain();
            let w_c = colour_remain(w_remain);
            footer_left += &format!("{DIM}7d{NC} {w_c}");
            footer_left_compact += &format!("{DIM}7d{NC} {w_c}");
            if let Some(reset) = w.resets_at {
                let rs = fmt_reset(reset);
                let fruit = pick_fruit(week_reset_slot.unwrap_or(0));
                let fs = if ne { HS } else { "" };
                footer_left += &format!(" {fruit}{fs}{DIM}{rs}{NC}");
            }
        }

        if let Some(f) = five {
            if !footer_left.is_empty() {
                footer_left += "  ";
            }
            let f_remain = f.remain();
            let f_c = colour_remain(f_remain);
            footer_left += &format!("{DIM}5h{NC} {f_c}");
            if !footer_left_compact.is_empty() {
                footer_left_compact += "  ";
            }
            footer_left_compact += &format!("{DIM}5h{NC} {f_c}");
            if let Some(reset) = f.resets_at {
                let rs = fmt_reset(reset);
                let fruit = pick_fruit(five_reset_slot.unwrap_or(0));
                let fs = if ne { HS } else { "" };
                footer_left += &format!(" {fruit}{fs}{DIM}{rs}{NC}");
            }
        }

        let (footer_right, footer_right_compact) = if rate_hit {
            ("Ha! ▶️".to_string(), "Ha! ▶️".to_string())
        } else {
            let effort = read_effort_level().filter(|s| !s.is_empty());
            let full = match effort {
                Some(e) if e == "xhigh" || e == "max" => {
                    format!("{DIM}{e}{NC}⏩ ✨Bloom!⏹️")
                }
                Some(e) => format!("{DIM}{e}{NC} ✨Bloom!⏹️"),
                None => "✨Bloom!⏹️".to_string(),
            };
            (full, "✨Bloom!⏹️".to_string())
        };

        let footer = [
            (&footer_left, &footer_right),
            (&footer_left_compact, &footer_right),
            (&footer_left_compact, &footer_right_compact),
            (&String::new(), &footer_right_compact),
        ]
        .into_iter()
        .find_map(|(left, right)| align_status_line(left, right, map_w, ne))
        .unwrap_or_else(|| pad_to_width_ex(&footer_right_compact, map_w, ne));

        // ── Combine: pad all lines to uniform visible width ─────────
        let target = map_w;
        let bp = self.buddy_padding;
        let top_pad = if bp {
            let blank: String = (0..target).map(|_| format!("{DIM}·{NC}")).collect();
            format!("{blank}\n")
        } else {
            String::new()
        };
        let mut result = format!(
            "{}{}\n{}\n{}\n{}\n{}",
            top_pad,
            pad_to_width_ex(&header, target, ne),
            pad_to_width_ex(&sky, target, ne),
            pad_to_width_ex(&game, target, ne),
            pad_to_width_ex(&grass, target, ne),
            pad_to_width_ex(&footer, target, ne),
        );
        if bp {
            let blank: String = (0..target).map(|_| format!("{DIM}·{NC}")).collect();
            for _ in 0..5 {
                result.push('\n');
                result.push_str(&blank);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ContextWindow, ModelField, RateLimit, RateLimits};

    #[test]
    fn test_pick_deterministic() {
        // Same slot always gives same flower
        assert_eq!(pick_small(100), pick_small(100));
        assert_eq!(pick_big(7), pick_big(7));
        assert_eq!(pick_fruit(42), pick_fruit(42));
    }

    #[test]
    fn test_trail_content_consumed_big() {
        let mut state = RawState::default();
        // slot 7: pos = (7+1)%8 = 0 → big flower
        state.slots.insert("7".to_string(), crate::models::SlotRecord { consumed: true });
        assert_eq!(trail_content(7, &state), pick_big(7));
    }

    #[test]
    fn test_trail_content_consumed_small() {
        let mut state = RawState::default();
        // slot 8: pos = (8+1)%8 = 1 → small flower
        state.slots.insert("8".to_string(), crate::models::SlotRecord { consumed: true });
        assert_eq!(trail_content(8, &state), pick_small(8));
    }

    #[test]
    fn test_trail_content_not_consumed_big_slot() {
        let mut state = RawState::default();
        // slot 7: pos 0, not consumed → seedling
        state.slots.insert("7".to_string(), crate::models::SlotRecord { consumed: false });
        assert_eq!(trail_content(7, &state), "🌱");
    }

    #[test]
    fn test_trail_content_not_consumed_normal() {
        let mut state = RawState::default();
        // slot 8: pos 1, not consumed → dot
        state.slots.insert("8".to_string(), crate::models::SlotRecord { consumed: false });
        assert_eq!(trail_content(8, &state), "·");
    }

    #[test]
    fn test_future_fruit_overrides() {
        let content = future_content(100, Some(100), None);
        assert_eq!(content, pick_fruit(100));
    }

    #[test]
    fn test_future_no_fruit_big_slot() {
        // slot 7: pos 0, no fruit → seedling
        let content = future_content(7, None, None);
        assert_eq!(content, "🌱");
    }

    #[test]
    fn test_future_no_fruit_normal() {
        // slot 8: pos 1, no fruit → dot
        let content = future_content(8, None, None);
        assert_eq!(content, "·");
    }

    #[test]
    fn test_render_slot_wide_with_flower_narrow() {
        let result = render_slot("🌻", 0, true);
        assert!(result.contains("🌻"));
        assert!(result.contains("\x1b[8m")); // hidden spacer for narrow emoji
    }

    #[test]
    fn test_render_slot_wide_with_flower_normal() {
        let result = render_slot("🌻", 0, false);
        assert!(result.contains("🌻"));
        assert!(!result.contains("\x1b[8m")); // no hidden spacer
    }

    #[test]
    fn test_render_slot_wide_empty() {
        let result = render_slot("·", 0, false);
        assert!(result.contains("·"));
        assert!(!result.contains("\x1b[8m"));
    }

    #[test]
    fn test_render_slot_normal() {
        let result = render_slot("🌸", 3, false);
        assert_eq!(result, "🌸");
    }

    #[test]
    fn test_narrow_width_reduces_trail() {
        let input = Input::default();
        let state = RawState::default();
        let theme = Pikmin {
            narrow_emoji: false,
            buddy_padding: false,
        };

        let narrow = theme.render(&input, &state, 30);
        let wide = theme.render(&input, &state, 100);

        let narrow_game = narrow.lines().nth(2).unwrap();
        let wide_game = wide.lines().nth(2).unwrap();
        assert!(visible_width(narrow_game) < visible_width(wide_game));
        assert_eq!(visible_width(narrow_game), 30);
        assert_eq!(visible_width(wide_game), 100);
    }

    #[test]
    fn test_all_lines_fit_width_with_full_status_content() {
        let input = Input {
            model: ModelField::Obj {
                display_name: Some("Opus".to_string()),
                id: None,
            },
            version: Some("2.1.0".to_string()),
            context_window: ContextWindow {
                used_percentage: 50.0,
                context_window_size: Some(1_000_000),
                total_tokens: None,
            },
            rate_limits: RateLimits {
                five_hour: Some(RateLimit {
                    used_percentage: 50.0,
                    resets_at: Some(u64::MAX),
                }),
                seven_day: Some(RateLimit {
                    used_percentage: 50.0,
                    resets_at: Some(u64::MAX),
                }),
            },
        };
        let theme = Pikmin {
            narrow_emoji: false,
            buddy_padding: false,
        };

        let output = theme.render(&input, &RawState::default(), 50);

        assert!(output.contains("50%"));
        assert!(output.contains("left"));
        assert!(output.contains("Bloom!"));
        for line in output.lines() {
            assert!(
                visible_width(line) <= 50,
                "line width {} exceeded 50: {line:?}",
                visible_width(line),
            );
        }
    }
}
