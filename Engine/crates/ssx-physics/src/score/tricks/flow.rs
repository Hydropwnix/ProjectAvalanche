use super::*;

pub(super) const END_15: &[(f32, f32)] = &[(ENTER_TICKS, 0.0)];
pub(super) const END_30: &[(f32, f32)] = &[(30.0, 0.0)];
pub(super) const EXIT_10: &[(f32, f32)] = &[(EXIT_TICKS, 2.0)];
pub(super) const EXIT_15: &[(f32, f32)] = &[(15.0, 2.0)];
pub(super) const EXIT_20: &[(f32, f32)] = &[(20.0, 2.0)];
pub(super) const EXIT_30: &[(f32, f32)] = &[(30.0, 2.0)];
pub(super) const ANTICIPATE_END: &[(f32, f32)] = &[(ANTICIPATE_TICKS, 0.0)];
pub(super) const AIR_JUMP_END: &[(f32, f32)] = &[(15.0, 15.0)];

impl TrickState {
    pub(super) fn node_rate(n: Node, ctx: &TrickContext, end: bool) -> f32 {
        use TrickLevel::*;
        let scale = trick_time_scale(ctx.ticks_to_land);
        match n {
            Node::Exit(SuperUber) | Node::TweakExit(SuperUber) | Node::SigExit(_) => scale,
            Node::Enter(Signature) | Node::Loop(Signature) if end => scale,
            _ => 1.0,
        }
    }

    pub(super) fn transitions(&self, n: Node, ctx: &TrickContext, end: bool) -> [Option<Transition<'_>>; 4] {
        use Node::*;
        use TrickLevel::*;
        let (start, tweaking, sig) = (self.has_valid_input, self.is_tweaking, &self.signature);
        let t = |to: Node, w: Option<&'static [(f32, f32)]>, c: bool| Some((to, w, c));
        let ok = !ctx.abort;
        match n {
            AirStates => [
                t(UberCrashRecover, None, ctx.recovering),
                t(AirJump, None, ctx.air_jump),
                t(Anticipate, None, ctx.enable_air_tricks && !ctx.board_particle && start && ok),
                None,
            ],
            UberCrashRecover => [t(AirStates, None, !ctx.recovering), None, None, None],
            AirJump => [t(AirStates, Some(AIR_JUMP_END), true), None, None, None],
            Anticipate => [
                t(Enter(Signature), Some(ANTICIPATE_END), self.is_signature),
                t(Enter(Uber), Some(ANTICIPATE_END), ctx.awesome),
                t(Enter(Normal), Some(ANTICIPATE_END), !ctx.awesome && ok),
                None,
            ],
            Enter(Normal) => [
                t(Exit(Normal), Some(END_15), ctx.abort),
                t(Exit(Normal), None, !start),
                t(TweakEnter(Normal), None, tweaking && ok),
                t(Loop(Normal), Some(END_15), ok),
            ],
            Loop(Normal) => [
                t(Exit(Normal), None, ctx.abort),
                t(TweakEnter(Normal), None, tweaking && ok),
                t(Exit(Normal), None, end),
                None,
            ],
            Exit(Normal) => [t(AirStates, Some(EXIT_10), true), t(Anticipate, None, start && ok), None, None],
            TweakEnter(Normal) => {
                [t(TweakExit(Normal), Some(END_15), ctx.abort), t(TweakLoop(Normal), Some(END_15), ok), None, None]
            }
            TweakExit(Normal) => [t(AirStates, Some(EXIT_10), true), None, None, None],
            Enter(Uber) => [t(Exit(Uber), None, !start), t(Loop(Uber), Some(END_15), true), None, None],
            Loop(Uber) => [
                t(Enter(SuperUber), None, ctx.super_awesome),
                t(TweakEnter(Uber), None, tweaking),
                t(Exit(Uber), None, end),
                None,
            ],
            Exit(Uber) => [t(AirStates, Some(EXIT_15), true), t(Anticipate, None, start), None, None],
            TweakEnter(Uber) => [t(TweakLoop(Uber), Some(END_30), true), None, None, None],
            TweakExit(Uber) => [t(AirStates, Some(EXIT_20), true), None, None, None],
            Enter(SuperUber) => [t(Exit(SuperUber), None, !start), t(Loop(SuperUber), Some(END_30), true), None, None],
            Loop(SuperUber) => [t(TweakEnter(SuperUber), None, tweaking), t(Exit(SuperUber), None, end), None, None],
            Exit(SuperUber) => [t(AirStates, Some(EXIT_30), true), t(Anticipate, None, start), None, None],
            TweakEnter(SuperUber) => [t(TweakLoop(SuperUber), Some(END_30), true), None, None, None],
            TweakExit(SuperUber) => {
                let short = matches!(self.trick_type_locked.flow_base(), TrickType::Rocket | TrickType::Tail);
                [t(AirStates, Some(if short { EXIT_20 } else { EXIT_30 }), true), None, None, None]
            }
            TweakLoop(Normal) => {
                [t(TweakExit(Normal), None, ctx.abort), t(TweakExit(Normal), None, !tweaking), None, None]
            }
            TweakLoop(l) => [t(TweakExit(l), None, !tweaking), None, None, None],
            Enter(Signature) => [
                Some((SigExit(1), Some(std::slice::from_ref(&sig.out_blend_1)), end)),
                Some((SigExit(2), Some(std::slice::from_ref(&sig.out_blend_2)), end)),
                Some((Loop(Signature), Some(std::slice::from_ref(&sig.enter_end)), self.is_signature)),
                None,
            ],
            Loop(Signature) => [t(SigExit(0), Some(sig.loop_ends), !self.gestures.signature || end), None, None, None],
            SigExit(0) => [Some((AirStates, Some(std::slice::from_ref(&sig.exit_end)), true)), None, None, None],
            SigExit(_) => [t(AirStates, Some(EXIT_30), true), None, None, None],
            Exit(Signature) | TweakEnter(Signature) | TweakExit(Signature) => [None, None, None, None],
        }
    }

    pub(super) fn state_flow(&mut self, ctx: &TrickContext, dt: f32) {
        let end = self.pad_grab_end();
        if self.node() != Node::AirStates {
            self.trick_ticks += dt;
        }
        let mut left = dt;
        for _ in 0..MAX_TRANSITIONS_PER_UPDATE {
            let node = self.node();
            self.rate = Self::node_rate(node, ctx, end);
            let Some((time, to)) = self.next_transition(node, ctx, end) else { break };
            let real = time / self.rate;
            if left <= real {
                break;
            }
            self.advance(time);
            left -= real;
            self.go(to, left, ctx, end);
        }
        if left > MIN_NODE_UPDATE_TICKS && self.node() != Node::AirStates {
            self.advance(left * self.rate);
        }
    }

    pub(super) fn next_transition(&self, node: Node, ctx: &TrickContext, end: bool) -> Option<(f32, Node)> {
        let tau = self.phase_ticks;
        let mut best: Option<(f32, Node)> = None;
        for (to, windows, cond) in self.transitions(node, ctx, end).into_iter().flatten() {
            if !cond {
                continue;
            }
            let time = match windows {
                None => Some(0.0),
                Some(w) => w
                    .iter()
                    .filter(|(time, len)| tau <= time + len)
                    .map(|(time, _)| (time - tau).max(0.0))
                    .min_by(f32::total_cmp),
            };
            if let Some(time) = time
                && best.is_none_or(|(b, _)| time < b)
            {
                best = Some((time, to));
                if time == 0.0 {
                    break;
                }
            }
        }
        best
    }

    pub(super) fn advance(&mut self, d: f32) {
        self.phase_ticks += d;
        if self.node() == Node::Loop(TrickLevel::Signature) && self.signature.loop_len > 0.0 {
            self.phase_ticks = self.phase_ticks.rem_euclid(self.signature.loop_len);
        }
    }

    pub(super) fn go(&mut self, to: Node, remaining: f32, ctx: &TrickContext, end: bool) {
        if matches!(self.node(), Node::Exit(_) | Node::SigExit(_) | Node::TweakExit(_)) {
            self.grab_reset();
        }
        match to {
            Node::AirStates => self.grab_reset(),
            Node::Anticipate => {
                self.input_index_on_enter = self.input_index;
                self.trick_ticks = remaining;
            }
            _ => {}
        }
        self.set_node(to);
        self.rate = Self::node_rate(to, ctx, end);
        self.phase_ticks = 0.0;
    }
}
