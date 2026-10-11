//! The Qwen soft switch — `/think` or `/no_think` on the end of the user's turn.
//!
//! WHY IT LIVES HERE AND NOT IN THE CLI. The rule was a private function in
//! `tacet-cli`'s `main.rs`, and the eval — which exists to measure the program a
//! user runs — had no way to reach it. So every ChatML model in the model table
//! was measured WITHOUT the suffix the shell appends to every turn: a hybrid
//! Qwen3 build ran in its default thinking mode in the eval, and under the
//! default `thinking = auto` the shell gives it `/no_think` on every turn the
//! rule does not call heavy. The cost of that difference has NOT been measured:
//! the 237-against-19 token medians in `tacet-eval`'s `tool_selection.rs`
//! compare the hybrid build with the instruct build — two different models —
//! not one model in its two modes. One rule, in the crate both sides already
//! depend on, is the only way the two stop drifting.
//!
//! WHAT IS NOT HERE: reading the user's setting. `thinking = on|off|auto` is a
//! CLI config key, and this crate has no business opening a config file; the
//! caller parses it with `ThinkingSetting::parse` and passes it in. The eval
//! passes `Auto`, the default, because the number it publishes is the number a
//! user with no config gets.

use crate::prompt::Template;

/// What the user asked for. `Auto` is the default and decides per message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkingSetting {
    On,
    Off,
    #[default]
    Auto,
}

impl ThinkingSetting {
    /// The config value as the CLI stores it. Anything that is not exactly `on`
    /// or `off` is `Auto` — the same reading the CLI's `match` gave it, so an
    /// unrecognised value never silently forces a mode.
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("on") => Self::On,
            Some("off") => Self::Off,
            _ => Self::Auto,
        }
    }
}

/// Messages that ask for real deliberation: planning, summarising, comparing,
/// explaining a cause. Matched against the FOLDED message (see `fold`), so the
/// Turkish entries are written without their diacritics.
const HEAVY: [&str; 12] = [
    "plan",
    "ozetle",
    "summar",
    "analiz",
    "analy",
    "karsilastir",
    "compare",
    "strateji",
    "strategy",
    "neden",
    "why",
    "adim adim",
];

/// Past this many characters a message is treated as heavy whatever it says:
/// a long request is usually a compound one.
const LONG_MESSAGE_CHARS: usize = 220;

/// The suffix to append to the user's turn, or `""` when the template has no
/// soft switch.
///
/// Thinking multiplies latency on a simple tool turn and buys little; on a
/// genuine planning or summarising turn it buys real quality. Non-ChatML
/// engines get no suffix at all — the soft switch is a Qwen convention, not a
/// standard, and another family would read `/no_think` as part of the question.
pub fn soft_switch(template: Template, setting: ThinkingSetting, message: &str) -> &'static str {
    if template != Template::ChatML {
        return "";
    }
    match setting {
        ThinkingSetting::On => " /think",
        ThinkingSetting::Off => " /no_think",
        ThinkingSetting::Auto => {
            let plain = fold(message);
            let heavy = message.chars().count() > LONG_MESSAGE_CHARS
                || HEAVY.iter().any(|k| plain.contains(k));
            if heavy { " /think" } else { " /no_think" }
        }
    }
}

/// Lowercase with the Turkish letters folded to ASCII, so "Özetle" matches
/// `ozetle`.
///
/// A COPY OF `tacet_tools::router::simplify`, and deliberately so: this crate
/// sits below `tacet-tools` and cannot call it. The shell and the eval both
/// call THIS fold now (the shell's private rule used the router's), so they
/// cannot disagree with each other; the copy is kept equal to the router's so
/// a message the router reads as "özetle" is read the same way here —
/// `tacet-eval`'s `the_thinking_switch_is_the_shells` runs both over every
/// message in the suite and fails if they ever differ.
pub fn fold(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        let d = match c {
            'ı' | 'İ' | 'I' => 'i',
            'ş' | 'Ş' => 's',
            'ğ' | 'Ğ' => 'g',
            'ü' | 'Ü' => 'u',
            'ö' | 'Ö' => 'o',
            'ç' | 'Ç' => 'c',
            'â' | 'Â' => 'a',
            'î' | 'Î' => 'i',
            'û' | 'Û' => 'u',
            _ => c,
        };
        s.extend(d.to_lowercase());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_tool_turn_is_told_not_to_think() {
        assert_eq!(
            soft_switch(Template::ChatML, ThinkingSetting::Auto, "What is 2 plus 2?"),
            " /no_think"
        );
    }

    #[test]
    fn a_planning_turn_is_told_to_think_in_either_language() {
        for m in [
            "Plan my week",
            "Bu raporu özetle",
            "ADIM ADIM anlat",
            "Why did the build fail?",
        ] {
            assert_eq!(
                soft_switch(Template::ChatML, ThinkingSetting::Auto, m),
                " /think",
                "{m}"
            );
        }
    }

    #[test]
    fn a_long_message_thinks_whatever_it_says() {
        let long = "add this row to the table ".repeat(10);
        assert!(long.chars().count() > LONG_MESSAGE_CHARS);
        assert_eq!(
            soft_switch(Template::ChatML, ThinkingSetting::Auto, &long),
            " /think"
        );
    }

    #[test]
    fn the_setting_overrides_the_heuristic() {
        assert_eq!(
            soft_switch(Template::ChatML, ThinkingSetting::On, "hi"),
            " /think"
        );
        assert_eq!(
            soft_switch(Template::ChatML, ThinkingSetting::Off, "plan it"),
            " /no_think"
        );
        assert_eq!(ThinkingSetting::parse(Some("on")), ThinkingSetting::On);
        assert_eq!(ThinkingSetting::parse(Some("off")), ThinkingSetting::Off);
        assert_eq!(ThinkingSetting::parse(Some("maybe")), ThinkingSetting::Auto);
        assert_eq!(ThinkingSetting::parse(None), ThinkingSetting::Auto);
    }

    /// The switch is a Qwen convention. Any other template reads it as text.
    #[test]
    fn only_chatml_gets_a_suffix() {
        for t in [Template::Plain, Template::Gemma] {
            for s in [
                ThinkingSetting::On,
                ThinkingSetting::Off,
                ThinkingSetting::Auto,
            ] {
                assert_eq!(soft_switch(t, s, "plan my week"), "", "{t:?} {s:?}");
            }
        }
    }
}
