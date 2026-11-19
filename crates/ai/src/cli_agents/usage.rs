//! Plan-quota usage for an agent that reports it directly through its own CLI (how much of the
//! person's session and week is used), as distinct from `claude_cost`'s per-turn API-style dollar
//! cost. Asked live, like model discovery: never cached or persisted. Claude Code and Antigravity
//! are the two agents that offer this today; the parsing was ported from the Genie app's own
//! usage checks.

use super::program_of;
use crate::{BackendConfig, BackendOptions};
use craft_anything_evidence::ToolError;
use craft_anything_platform::Process;
use std::time::Duration;

const USAGE_TIMEOUT: Duration = Duration::from_secs(20);

/// One agent's plan-quota reading. `None` means that line wasn't in its output, never 0%.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlanUsage {
    pub session_used_percent: Option<f64>,
    pub weekly_used_percent: Option<f64>,
}

impl PlanUsage {
    pub fn is_empty(&self) -> bool {
        self.session_used_percent.is_none() && self.weekly_used_percent.is_none()
    }
}

/// The number right before the first `%` in `text`, read backwards from it.
fn percent_in(text: &str) -> Option<f64> {
    let idx = text.find('%')?;
    let digits: String = text[..idx].chars().rev().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<Vec<_>>().into_iter().rev().collect();
    digits.parse().ok()
}

/// `claude -p "/cost"` prints lines like "Current session: 100% used · resets …" and
/// "Current week (all models): 44% used · resets …".
pub(crate) fn parse_claude_cost(text: &str) -> PlanUsage {
    let mut usage = PlanUsage::default();
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Current session:") {
            usage.session_used_percent = percent_in(rest);
        } else if let Some(rest) = line.strip_prefix("Current week").and_then(|l| l.split_once(':')).map(|(_, r)| r) {
            usage.weekly_used_percent = percent_in(rest);
        }
    }
    usage
}

/// `agy --print "/usage"` prints the remaining percent of each quota, by model group, e.g.
/// "Gemini Models  Five Hour Limit Remaining  88%  …". Only the Gemini Models row is this
/// backend's own quota; the "Claude and GPT models" row belongs to whatever those cost there.
pub(crate) fn parse_antigravity_usage(text: &str) -> PlanUsage {
    let mut usage = PlanUsage::default();
    for line in text.lines().map(str::trim) {
        if !line.starts_with("Gemini Models") {
            continue;
        }
        if line.contains("Five Hour Limit Remaining") {
            usage.session_used_percent = percent_in(line).map(|remaining| (100.0 - remaining).clamp(0.0, 100.0));
        } else if line.contains("Weekly Limit Remaining") {
            usage.weekly_used_percent = percent_in(line).map(|remaining| (100.0 - remaining).clamp(0.0, 100.0));
        }
    }
    usage
}

/// Ask Claude Code how much of the person's plan its session and week have used so far.
pub fn fetch_claude_usage(config: &BackendConfig, options: &BackendOptions) -> Result<PlanUsage, ToolError> {
    let program = program_of(config, options)?;
    let out = Process::new(program).arg("-p").arg("/cost").timeout(USAGE_TIMEOUT).run()?;
    Ok(parse_claude_cost(&out.stdout_text()))
}

/// Ask Antigravity how much of the person's Gemini-models plan its session and week have used.
pub fn fetch_antigravity_usage(config: &BackendConfig, options: &BackendOptions) -> Result<PlanUsage, ToolError> {
    let program = program_of(config, options)?;
    let out = Process::new(program).arg("--print").arg("/usage").timeout(USAGE_TIMEOUT).run()?;
    Ok(parse_antigravity_usage(&out.stdout_text()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_cost_output() {
        let sample = "\
You are currently using your subscription to power your Claude Code usage

Current session: 100% used · resets Nov 17 at 8:09pm (Asia/Seoul)
Current week (all models): 44% used · resets Nov 21 at 12:59pm (Asia/Seoul)

What's contributing to your usage?
";
        let usage = parse_claude_cost(sample);
        assert_eq!(usage.session_used_percent, Some(100.0));
        assert_eq!(usage.weekly_used_percent, Some(44.0));
    }

    #[test]
    fn parses_antigravity_usage_output() {
        let sample = "\
Quota:
Gemini Models          Weekly Limit Remaining     91%   2026-11-23 13:19 KST
Gemini Models          Five Hour Limit Remaining  88%   2026-11-17 21:27 KST
Claude and GPT models  Weekly Limit Remaining     100%  2026-11-24 17:46 KST
Claude and GPT models  Five Hour Limit Remaining  100%  2026-11-17 22:46 KST
";
        let usage = parse_antigravity_usage(sample);
        assert_eq!(usage.weekly_used_percent, Some(9.0));
        assert_eq!(usage.session_used_percent, Some(12.0));
    }

    #[test]
    fn unrecognised_text_leaves_both_fields_unknown_not_zero() {
        assert_eq!(parse_claude_cost("some random text"), PlanUsage::default());
        assert_eq!(parse_antigravity_usage("some random text"), PlanUsage::default());
    }
}
