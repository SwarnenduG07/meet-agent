//! The orchestrator: one state at a time, only the actions that state allows, and
//! the presentation plan. It drives the Meet call; the conversation loop talks.

use std::time::Instant;

use anyhow::Result;
use meet::call::Call;
use serde_json::json;

use crate::{
    conversation::{Control, Director, Flow, Tools, log},
    plan::{Plan, SharePolicy},
    timeline::Timeline,
};

/// What the agent is doing in the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// In the call, answering questions.
    Listening,
    /// Walking through the plan, section by section.
    Presenting,
    /// Presenting, but someone is talking or asking.
    Paused,
    /// You took over the call.
    TakenOver,
}

impl State {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Listening => "LISTENING",
            Self::Presenting => "PRESENTING",
            Self::Paused => "PAUSED",
            Self::TakenOver => "TAKEN_OVER",
        }
    }
}

/// The policy: which actions each state allows.
#[must_use]
pub fn allowed(
    state: State,
    sharing: bool,
    policy: SharePolicy,
    has_plan: bool,
) -> Vec<&'static str> {
    let mut a = vec!["answer", "defer_to_sunny", "wait"];
    match state {
        State::Listening => {
            if policy != SharePolicy::Never {
                a.push(if sharing {
                    "stop_sharing"
                } else {
                    "share_screen"
                });
            }
            if has_plan {
                a.push("start_presenting");
            }
            a.push("leave");
        }
        State::Presenting | State::Paused => {
            a.extend(["resume", "next_section", "stop_presenting"]);
            if sharing {
                a.push("stop_sharing");
            }
            a.push("leave");
        }
        State::TakenOver => {}
    }
    a
}

pub struct Orchestrator {
    /// `None` in a rehearsal: everything runs except the Meet clicks.
    pub call: Option<Call>,
    pub plan: Plan,
    pub timeline: Timeline,
    pub owner: String,
    pub agent_mic: String,
    pub your_mic: String,
    pub state: State,
    pub sharing: bool,
    pub left: bool,
    /// Section being presented and the next sentence of its script.
    section: usize,
    sentence: usize,
    paused_at: Option<Instant>,
}

impl Orchestrator {
    #[must_use]
    pub fn new(
        call: Option<Call>,
        plan: Plan,
        timeline: Timeline,
        owner: String,
        agent_mic: String,
        your_mic: String,
    ) -> Self {
        Self {
            call,
            plan,
            timeline,
            owner,
            agent_mic,
            your_mic,
            state: State::Listening,
            sharing: false,
            left: false,
            section: 0,
            sentence: 0,
            paused_at: None,
        }
    }

    fn set_state(&mut self, state: State) {
        if self.state != state {
            log(
                "state",
                &format!("{} -> {}", self.state.name(), state.name()),
            );
            self.timeline.record(
                "state",
                json!({ "from": self.state.name(), "to": state.name() }),
            );
            self.state = state;
        }
    }

    async fn share(&mut self) -> Result<()> {
        if self.plan.share == SharePolicy::Never {
            return Ok(());
        }
        if let Some(call) = &self.call {
            call.share().await?;
        }
        self.sharing = true;
        self.timeline.record("share", json!({}));
        log("meet", "sharing the screen");
        self.show_current();
        Ok(())
    }

    async fn stop_sharing(&mut self) -> Result<()> {
        if let Some(call) = &self.call {
            call.stop_sharing().await?;
        }
        self.sharing = false;
        self.timeline.record("stop_sharing", json!({}));
        log("meet", "stopped sharing");
        Ok(())
    }

    /// Brings the current section's material to the front.
    fn show_current(&mut self) {
        let Some(target) = self
            .plan
            .sections
            .get(self.section)
            .and_then(|s| s.show.clone())
        else {
            return;
        };
        match desktop::windows::show(&target) {
            Ok(()) => log("screen", &format!("showing {target}")),
            Err(e) => log("screen", &format!("couldn't show {target}: {e}")),
        }
    }

    /// Starts the plan right after joining (share policy `at_start`).
    pub async fn start_presenting_now(&mut self) -> Result<()> {
        self.start_presenting().await
    }

    async fn start_presenting(&mut self) -> Result<()> {
        if self.plan.sections.is_empty() {
            return Ok(());
        }
        if !self.sharing && self.plan.share != SharePolicy::Never {
            self.share().await?;
        }
        self.section = 0;
        self.sentence = 0;
        self.begin_section();
        self.set_state(State::Presenting);
        Ok(())
    }

    fn begin_section(&mut self) {
        if let Some(s) = self.plan.sections.get(self.section) {
            log("section", &format!("{}. {}", self.section + 1, s.title));
            self.timeline.record(
                "section",
                json!({ "index": self.section, "title": s.title }),
            );
        }
        self.show_current();
    }

    fn next_section(&mut self) {
        if let Some(s) = self.plan.sections.get(self.section) {
            self.timeline.covered.push(s.title.clone());
        }
        self.section += 1;
        self.sentence = 0;
        if self.section < self.plan.sections.len() {
            self.begin_section();
        }
    }

    /// Queues the next sentence of the script when the previous one is nearly done.
    async fn present_step(&mut self, tools: &Tools<'_>) -> Result<()> {
        if tools.voice.queued_secs() > 0.6 {
            return Ok(());
        }
        let Some(section) = self.plan.sections.get(self.section) else {
            // Finished the plan.
            tools
                .voice
                .say("That's everything I wanted to show. Happy to take any questions.")
                .await?;
            self.set_state(State::Listening);
            return Ok(());
        };
        if let Some(line) = section.script.get(self.sentence).cloned() {
            self.sentence += 1;
            log("present", &line);
            tools.voice.say(&line).await?;
        } else if !tools.voice.is_speaking() {
            self.next_section();
        }
        Ok(())
    }

    async fn toggle_take_over(&mut self, tools: &Tools<'_>) -> Result<()> {
        if self.state == State::TakenOver {
            let mic = self.switch_mic(&self.agent_mic.clone()).await?;
            self.set_state(State::Listening);
            log("control", &format!("agent is back (mic: {mic})"));
        } else {
            tools.voice.stop();
            let mic = self.switch_mic(&self.your_mic.clone()).await?;
            self.set_state(State::TakenOver);
            log(
                "control",
                &format!("you have the call (mic: {mic}); type t to hand it back"),
            );
        }
        Ok(())
    }

    async fn switch_mic(&self, device: &str) -> Result<String> {
        match &self.call {
            Some(call) => call.switch_mic(device).await,
            None => Ok(format!("{device} (rehearsal)")),
        }
    }

    fn current_section_text(&self) -> String {
        match self.plan.sections.get(self.section) {
            Some(s) if matches!(self.state, State::Presenting | State::Paused) => format!(
                "\n\nCURRENTLY PRESENTING section {} of {}: {}\nTalking points: {}",
                self.section + 1,
                self.plan.sections.len(),
                s.title,
                s.talk.join(" ")
            ),
            _ => String::new(),
        }
    }
}

impl Director for Orchestrator {
    fn state(&self) -> &'static str {
        self.state.name()
    }

    fn allowed(&self) -> Vec<&'static str> {
        allowed(
            self.state,
            self.sharing,
            self.plan.share,
            !self.plan.sections.is_empty(),
        )
    }

    fn context(&self) -> String {
        let recent: Vec<String> = self
            .timeline
            .questions
            .iter()
            .rev()
            .take(6)
            .rev()
            .map(|(q, a)| format!("Q: {q}\nA: {a}"))
            .collect();
        let mut ctx = self.plan.context.clone();
        ctx.push_str(&self.current_section_text());
        if !recent.is_empty() {
            ctx.push_str("\n\nEARLIER IN THIS MEETING:\n");
            ctx.push_str(&recent.join("\n"));
        }
        ctx
    }

    fn listening(&self) -> bool {
        self.state != State::TakenOver
    }

    fn speech_started(&mut self) {
        if self.state == State::Presenting {
            self.set_state(State::Paused);
            self.paused_at = Some(Instant::now());
            // Repeat the interrupted sentence after the question.
            self.sentence = self.sentence.saturating_sub(1);
        }
    }

    async fn act(
        &mut self,
        question: &str,
        action: &str,
        say: &str,
        tools: &Tools<'_>,
    ) -> Result<Flow> {
        self.timeline.record(
            "turn",
            json!({ "heard": question, "action": action, "said": say }),
        );
        if !self.allowed().contains(&action) {
            log(
                "policy",
                &format!("refused {action} in {}", self.state.name()),
            );
            self.timeline.record(
                "refused",
                json!({ "action": action, "state": self.state.name() }),
            );
            tools
                .voice
                .say(&format!(
                    "Let me check that with {} and come back to you.",
                    self.owner
                ))
                .await?;
            return Ok(Flow::Continue);
        }
        if !say.is_empty() {
            self.timeline
                .questions
                .push((question.to_owned(), say.to_owned()));
        }
        match action {
            "defer_to_sunny" => self.timeline.deferred.push(question.to_owned()),
            "share_screen" => self.share().await?,
            "stop_sharing" => self.stop_sharing().await?,
            "start_presenting" => self.start_presenting().await?,
            "next_section" => {
                self.next_section();
                self.set_state(State::Presenting);
            }
            "stop_presenting" => self.set_state(State::Listening),
            "leave" => {
                tools.voice.finish().await;
                return Ok(Flow::Stop);
            }
            _ => {}
        }
        // A question during the presentation is answered: carry on.
        if self.state == State::Paused && matches!(action, "answer" | "defer_to_sunny" | "resume") {
            self.set_state(State::Presenting);
        }
        Ok(Flow::Continue)
    }

    async fn tick(&mut self, tools: &Tools<'_>, busy: bool) -> Result<Flow> {
        if let Some(call) = &self.call
            && !call.in_call().await
        {
            log("meet", "the call ended or the agent was removed");
            self.timeline.record("call_ended", json!({}));
            self.left = true;
            return Ok(Flow::Stop);
        }
        match self.state {
            State::Presenting if !busy => self.present_step(tools).await?,
            // Someone made a sound but asked nothing: carry on after a moment.
            State::Paused
                if !busy && self.paused_at.is_some_and(|t| t.elapsed().as_secs() >= 6) =>
            {
                self.set_state(State::Presenting);
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    async fn control(&mut self, control: Control, tools: &Tools<'_>) -> Result<Flow> {
        match control {
            Control::TakeOver => self.toggle_take_over(tools).await?,
            Control::Share if self.sharing => self.stop_sharing().await?,
            Control::Share => self.share().await?,
            Control::Next if matches!(self.state, State::Presenting | State::Paused) => {
                tools.voice.stop();
                self.next_section();
                self.set_state(State::Presenting);
            }
            Control::Next => self.start_presenting().await?,
            Control::Leave => {
                tools.voice.stop();
                tools
                    .voice
                    .say("Thanks everyone, I'll leave the call now.")
                    .await?;
                tools.voice.finish().await;
                return Ok(Flow::Stop);
            }
        }
        Ok(Flow::Continue)
    }

    async fn finish(&mut self, _tools: &Tools<'_>) -> Result<()> {
        if self.sharing && !self.left {
            let _ = self.stop_sharing().await;
        }
        Ok(())
    }
}
