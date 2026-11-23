//! Templates: ready-made workflows a person can try, customise and approve.
//!
//! A template is a draft, nothing more. It is built with the person's own
//! connector servers, default model and timezone, it grants only servers that
//! are configured, and it is saved unapproved: nothing runs until a person
//! approves it, exactly as with a workflow drafted from a sentence.

use bridge_protocol::messages as wire;

use crate::workflow_draft::DraftContext;

/// The apps a template can need, matched by name in a connector's id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum App {
    Slack,
    Gmail,
    Calendar,
    Notion,
    Linear,
}

impl App {
    fn label(self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::Gmail => "gmail",
            Self::Calendar => "google calendar",
            Self::Notion => "notion",
            Self::Linear => "linear",
        }
    }

    /// The words that must appear, in order, in a connector's id.
    fn words(self) -> &'static [&'static str] {
        match self {
            Self::Slack => &["slack"],
            Self::Gmail => &["gmail"],
            Self::Calendar => &["calendar"],
            Self::Notion => &["notion"],
            Self::Linear => &["linear"],
        }
    }

    fn matches(self, server: &str) -> bool {
        let words: Vec<String> = server
            .to_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_owned)
            .collect();
        self.words().iter().all(|needed| words.iter().any(|word| word == needed))
    }
}

/// How a template starts.
enum Starts {
    /// A cron expression, read in the person's timezone.
    Schedule(&'static str),
    Event(wire::WorkflowEventKind),
}

struct Spec {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    starts: Starts,
    reads: &'static [App],
    needs_project: bool,
    instructions: &'static str,
    expected: &'static str,
    notify_on_success: bool,
}

const SPECS: &[Spec] = &[
    Spec {
        id: "slack-morning-digest",
        name: "Morning Slack catch-up",
        description: "Every weekday at 9:00, what needs you in Slack, most urgent first.",
        starts: Starts::Schedule("0 9 * * 1-5"),
        reads: &[App::Slack],
        needs_project: false,
        instructions: "Find the Slack messages from the last 24 hours that mention me, message me directly, or reply in threads I am part of. \
            Group them by what they need from me (an answer, a review, a decision, or just to know), most urgent first, with a link to each. \
            Leave out anything I have already replied to. Do not send anything.",
        expected: "A short list of what needs me in Slack, most urgent first, with links.",
        notify_on_success: true,
    },
    Spec {
        id: "slack-mention-triage",
        name: "Triage each Slack mention",
        description: "When someone mentions you in Slack, what they need and how urgent it is.",
        starts: Starts::Event(wire::WorkflowEventKind::SlackMention),
        reads: &[App::Slack],
        needs_project: false,
        instructions: "Someone mentioned me in Slack; the event is below. Read the thread around it. \
            Then tell me in two or three lines what they need from me, how urgent it is, and a reply I could send. Do not reply yourself.",
        expected: "Two or three lines: the ask, how urgent it is, and a suggested reply.",
        notify_on_success: true,
    },
    Spec {
        id: "gmail-needs-reply",
        name: "Email that needs a reply",
        description: "Every weekday at 8:30, the unread email that needs an answer from you.",
        starts: Starts::Schedule("30 8 * * 1-5"),
        reads: &[App::Gmail],
        needs_project: false,
        instructions: "Look at my unread email from the last day. List the messages that need a reply from me, most important first, \
            each with the sender, what they are asking, and a one-line answer I could give. Leave out newsletters, receipts and notifications. Send nothing.",
        expected: "The emails that need a reply, most important first, each with a one-line suggested answer.",
        notify_on_success: true,
    },
    Spec {
        id: "calendar-day-prep",
        name: "Prepare for today's meetings",
        description: "Every weekday at 8:00, today's meetings and what to prepare for each.",
        starts: Starts::Schedule("0 8 * * 1-5"),
        reads: &[App::Calendar],
        needs_project: false,
        instructions: "List today's meetings from my calendar in order. For each, give the time, who is attending, what it is about, \
            and one thing I should prepare or decide before it. Point out any overlaps or meetings with no agenda.",
        expected: "Today's meetings in order, each with one thing to prepare.",
        notify_on_success: true,
    },
    Spec {
        id: "notion-weekly-changes",
        name: "What changed in Notion this week",
        description: "Every Monday at 9:00, the Notion pages that changed last week.",
        starts: Starts::Schedule("0 9 * * 1"),
        reads: &[App::Notion],
        needs_project: false,
        instructions: "Find the Notion pages created or edited in the last 7 days. Summarise the most important changes, grouped by area, \
            with a link to each page. Keep it to what a teammate coming back from a week away would need to know.",
        expected: "The week's important Notion changes, grouped by area, with links.",
        notify_on_success: true,
    },
    Spec {
        id: "linear-standup",
        name: "My Linear standup",
        description: "Every weekday at 9:00, your Linear issues that moved, and three lines for standup.",
        starts: Starts::Schedule("0 9 * * 1-5"),
        reads: &[App::Linear],
        needs_project: false,
        instructions: "List the Linear issues assigned to me that changed since yesterday, and any that are blocked or overdue. \
            Finish with three short lines I could say at standup: done, doing, blocked.",
        expected: "My moved, blocked and overdue issues, and three standup lines.",
        notify_on_success: true,
    },
    Spec {
        id: "github-review-digest",
        name: "Summarise each review request",
        description: "When your review is requested on GitHub, a summary of the pull request and where to look first.",
        starts: Starts::Event(wire::WorkflowEventKind::GithubReviewRequested),
        reads: &[],
        needs_project: true,
        instructions: "My review was requested on the pull request in the event below. Read it with `gh pr view` and `gh pr diff` \
            (use the number and repository from the event). Summarise what it changes, the riskiest part, and where I should look first. \
            Change no files and post nothing.",
        expected: "What the pull request changes, its riskiest part, and where to start reviewing.",
        notify_on_success: true,
    },
    Spec {
        id: "nightly-tests",
        name: "Nightly test run",
        description: "Every night at 2:00, run the project's tests in a fresh copy and report what failed.",
        starts: Starts::Schedule("0 2 * * *"),
        reads: &[],
        needs_project: true,
        instructions: "Run this project's test suite (find the command in the README, package.json, Makefile or CI configuration). \
            Report which tests failed and the most likely cause of each. Change no files.",
        expected: "Which tests failed and the likely cause of each, or that everything passed.",
        notify_on_success: false,
    },
];

/// The configured server a template would use for an app: a connected one
/// first, then any.
fn server_for<'a>(app: App, servers: &'a [wire::WorkflowServerOption]) -> Option<&'a wire::WorkflowServerOption> {
    let candidates: Vec<&wire::WorkflowServerOption> = servers.iter().filter(|server| app.matches(&server.id)).collect();
    candidates
        .iter()
        .copied()
        .find(|server| server.connected == Some(true))
        .or_else(|| candidates.first().copied())
}

fn build(spec: &Spec, context: &DraftContext, harness: &wire::HarnessId) -> wire::WorkflowTemplate {
    let mut missing = Vec::new();
    let mut grants = Vec::new();
    for app in spec.reads {
        match server_for(*app, &context.servers) {
            Some(server) => {
                if server.connected != Some(true) {
                    missing.push(app.label().to_owned());
                }
                grants.push(wire::WorkflowGrant { server: server.id.clone(), read: true, write: wire::WorkflowWriteAccess::Never });
            }
            None => missing.push(app.label().to_owned()),
        }
    }
    let trigger = match &spec.starts {
        Starts::Schedule(cron) => wire::WorkflowTrigger::Schedule { cron: (*cron).to_owned(), timezone: context.timezone.clone() },
        Starts::Event(event) => wire::WorkflowTrigger::Event { event: *event, repo: None, channel: None, from: None, batch: None },
    };
    let definition = wire::WorkflowDefinition {
        name: spec.name.to_owned(),
        project_id: None,
        steps: vec![wire::WorkflowStep {
            harness: harness.clone(),
            model: context.model.clone(),
            effort: None,
            instructions: spec.instructions.to_owned(),
        }],
        trigger,
        grants,
        always_allow: Vec::new(),
        files: if spec.needs_project { wire::WorkflowFiles::Worktree } else { wire::WorkflowFiles::None },
        budget: wire::WorkBriefLimits { max_wall_seconds: 600, max_turns: 24, max_tool_calls: 64, max_output_tokens: None, cost_ceiling_microusd: None },
        expected_result: spec.expected.to_owned(),
        notify: wire::WorkflowNotify { on_success: spec.notify_on_success },
    };
    wire::WorkflowTemplate {
        id: spec.id.to_owned(),
        name: spec.name.to_owned(),
        description: spec.description.to_owned(),
        needs: spec.reads.iter().map(|app| app.label().to_owned()).collect(),
        ready: missing.is_empty() && !spec.needs_project,
        missing,
        needs_project: spec.needs_project,
        definition,
    }
}

/// Every template, ready ones first, in the order they are listed.
pub fn templates(context: &DraftContext) -> Vec<wire::WorkflowTemplate> {
    // Bridge names the harness itself; one it cannot parse offers nothing.
    let Ok(harness) = wire::HarnessId::parse(&context.harness) else { return Vec::new() };
    let mut built: Vec<wire::WorkflowTemplate> = SPECS.iter().map(|spec| build(spec, context, &harness)).collect();
    built.sort_by_key(|template| !template.ready);
    built
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(id: &str, connected: Option<bool>) -> wire::WorkflowServerOption {
        wire::WorkflowServerOption { id: id.into(), family: "unknown".into(), connected }
    }

    fn context(servers: Vec<wire::WorkflowServerOption>) -> DraftContext {
        DraftContext { servers, timezone: "Asia/Tokyo".into(), harness: "claude".into(), model: "claude-haiku-5-5".into() }
    }

    #[test]
    fn a_template_is_ready_only_with_every_connector_signed_in_and_no_project_to_choose() {
        let templates = templates(&context(vec![
            server("claude.ai Slack", Some(true)),
            server("claude.ai Gmail", Some(false)),
            server("claude.ai Google Calendar", Some(true)),
        ]));
        let by_id = |id: &str| templates.iter().find(|template| template.id == id).unwrap();
        assert!(by_id("slack-morning-digest").ready);
        assert!(by_id("calendar-day-prep").ready, "Google Calendar is matched by name");
        let gmail = by_id("gmail-needs-reply");
        assert!(!gmail.ready);
        assert_eq!(gmail.missing, vec!["gmail"], "configured but signed out is missing");
        assert_eq!(gmail.definition.grants.len(), 1, "it still names the configured server, for Customize");
        let linear = by_id("linear-standup");
        assert_eq!(linear.missing, vec!["linear"]);
        assert!(linear.definition.grants.is_empty(), "never a grant on a server that is not configured");
        let tests = by_id("nightly-tests");
        assert!(tests.needs_project && !tests.ready, "a code template waits for a project");
        let ready_first: Vec<bool> = templates.iter().map(|template| template.ready).collect();
        assert!(ready_first.windows(2).all(|pair| pair[0] >= pair[1]), "ready templates come first");
    }

    #[test]
    fn templates_use_the_persons_timezone_and_model_and_grant_reads_only() {
        for template in templates(&context(vec![server("claude.ai Slack", Some(true)), server("Slack (work)", Some(true))])) {
            assert_eq!(template.definition.steps[0].model, "claude-haiku-5-5");
            if let wire::WorkflowTrigger::Schedule { timezone, .. } = &template.definition.trigger {
                assert_eq!(timezone, "Asia/Tokyo");
            }
            assert!(template.definition.grants.iter().all(|grant| grant.read && grant.write == wire::WorkflowWriteAccess::Never), "{}", template.id);
            assert!(template.definition.always_allow.is_empty());
        }
    }

    #[test]
    fn a_template_uses_the_connected_server_when_two_match() {
        let servers = vec![server("Slack (old)", Some(false)), server("claude.ai Slack", Some(true)), server("Slack (spare)", None)];
        let digest = templates(&context(servers)).into_iter().find(|template| template.id == "slack-morning-digest").unwrap();
        assert_eq!(digest.definition.grants.iter().map(|grant| grant.server.as_str()).collect::<Vec<_>>(), ["claude.ai Slack"]);
        // With none connected, the first that matches.
        let unknown = templates(&context(vec![server("Slack (old)", Some(false)), server("Slack (spare)", None)]));
        let digest = unknown.into_iter().find(|template| template.id == "slack-morning-digest").unwrap();
        assert_eq!(digest.definition.grants[0].server, "Slack (old)");
    }

    #[test]
    fn every_ready_template_passes_validation() {
        let servers = ["claude.ai Slack", "claude.ai Gmail", "claude.ai Google Calendar", "claude.ai Notion", "claude.ai Linear"];
        let context = context(servers.iter().map(|id| server(id, Some(true))).collect());
        let validation = crate::workflows::ValidationContext {
            adapters: vec![crate::workflows::AdapterFacts { id: "claude".into(), models: vec![] }],
            servers: servers.iter().map(|id| (*id).to_owned()).collect(),
            servers_unknown: None,
        };
        for template in templates(&context) {
            let mut definition = template.definition.clone();
            if template.needs_project {
                definition.project_id = Some("p".into());
                if let wire::WorkflowTrigger::Event { repo, .. } = &mut definition.trigger {
                    *repo = Some("acme/app".into());
                }
            }
            assert_eq!(crate::workflows::validate(&definition, &validation), Ok(()), "{}", template.id);
        }
    }

    #[test]
    fn a_server_is_matched_by_whole_words() {
        assert!(App::Slack.matches("claude.ai Slack"));
        assert!(App::Calendar.matches("claude.ai Google Calendar"));
        assert!(!App::Slack.matches("claude.ai Slackware"), "a longer word is another app");
        assert!(!App::Linear.matches("claude.ai Nonlinear"));
    }
}
