use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Objective {
    pub name: Name,
    pub nodes: Vec<Node>,
    pub cur: Option<usize>,
    pub restarted: bool,
    pub overridden: bool,
    pub script: String,
}

impl Objective {
    pub fn new(name: Name, script: &str) -> Self {
        Self { name, nodes: Vec::new(), cur: None, restarted: false, overridden: false, script: script.to_string() }
    }

    pub fn complete(&self) -> bool {
        self.cur == Some(self.nodes.len())
    }

    pub fn append(&mut self, n: Node) {
        match self.nodes.last_mut() {
            Some(last) if !last.complete() => last.set_operand(n),
            _ => self.nodes.push(n),
        }
    }
}

fn update_objective(nodes: &mut [Node], cur: &mut Option<usize>, restarted: &mut bool, cx: &mut Ctx, dt: f32) {
    *restarted = false;
    while let Some(i) = *cur
        && i < nodes.len()
    {
        if !nodes[i].test(cx, dt) {
            return;
        }
        nodes[i].execute(cx);
        if std::mem::take(&mut cx.restart) {
            restart_objective(nodes, cur, restarted, cx);
            return;
        }
        *cur = Some(i + 1);
        match nodes.get_mut(i + 1) {
            Some(n) => n.begin(cx),
            None => cx.logged(Logged::Completed),
        }
    }
}

fn restart_objective(nodes: &mut [Node], cur: &mut Option<usize>, restarted: &mut bool, cx: &mut Ctx) {
    let c = cur.unwrap_or(usize::MAX);
    for (k, n) in nodes.iter_mut().enumerate() {
        n.reset(cx, k <= c, k < c);
    }
    *cur = Some(0);
    if let Some(n) = nodes.first_mut() {
        n.begin(cx);
    }
    *restarted = true;
    cx.logged(Logged::Restarted);
}

impl Ctx<'_> {
    fn logged(&mut self, what: Logged) {
        if let Some(log) = self.log.as_deref_mut() {
            log.push(LogEntry { time: self.time, objective: self.objective.text.to_string(), what });
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetId {
    Game = 0,
    Level = 1,
    FrontEnd = 2,
}

#[derive(Clone, Debug, Default)]
pub struct Runner {
    pub sets: [Vec<Objective>; 3],
    pub flags: NamedFlags,
    pub events: crate::game::events::PostedEvents,
    pub level_active: bool,
    pub time: f64,
    pub unprovided: std::collections::BTreeSet<String>,
    pub log: Option<Vec<LogEntry>>,
}

pub fn find(sets: &[Vec<Objective>; 3], hash: u32) -> Option<(usize, usize)> {
    sets.iter().enumerate().find_map(|(s, v)| v.iter().position(|o| o.name.hash == hash).map(|i| (s, i)))
}

impl Runner {
    pub fn update(&mut self, dt: f32, world: &mut dyn World) {
        self.time += f64::from(dt);
        self.events.swap();
        let second = if self.level_active { 1 } else { 2 };
        for s in [0, second] {
            for i in 0..self.sets[s].len() {
                if self.sets[s][i].complete() {
                    continue;
                }
                self.with_objective(s, i, world, |nodes, cur, restarted, cx| {
                    update_objective(nodes, cur, restarted, cx, dt)
                });
            }
        }
    }

    fn with_objective(
        &mut self,
        s: usize,
        i: usize,
        world: &mut dyn World,
        f: impl FnOnce(&mut [Node], &mut Option<usize>, &mut bool, &mut Ctx),
    ) {
        let o = &mut self.sets[s][i];
        let mut nodes = std::mem::take(&mut o.nodes);
        let (mut cur, mut restarted, name) = (o.cur, o.restarted, o.name.clone());
        {
            let sets = &self.sets;
            let complete = |h: u32| find(sets, h).map(|(fs, fi)| (fs, fi) != (s, i) && sets[fs][fi].complete());
            let mut cx = Ctx {
                world,
                flags: &mut self.flags,
                events: &mut self.events,
                complete: &complete,
                objective: &name,
                restart: false,
                unprovided: &mut self.unprovided,
                log: self.log.as_mut(),
                time: self.time,
            };
            f(&mut nodes, &mut cur, &mut restarted, &mut cx);
        }
        let o = &mut self.sets[s][i];
        (o.nodes, o.cur, o.restarted) = (nodes, cur, restarted);
    }

    pub fn start(&mut self, s: usize, i: usize, world: &mut dyn World) {
        self.with_objective(s, i, world, |nodes, cur, _, cx| {
            *cur = Some(0);
            if let Some(n) = nodes.first_mut() {
                n.begin(cx);
            }
        });
    }

    pub fn restart_set(&mut self, set: SetId, world: &mut dyn World) {
        let s = set as usize;
        for i in 0..self.sets[s].len() {
            self.with_objective(s, i, world, restart_objective);
        }
    }

    pub fn clear(&mut self, s: usize, i: usize, world: &mut dyn World) {
        self.with_objective(s, i, world, |nodes, cur, restarted, cx| {
            let c = cur.unwrap_or(usize::MAX);
            for (k, n) in nodes.iter_mut().enumerate() {
                n.reset(cx, k <= c, k < c);
            }
            (*cur, *restarted) = (None, false);
        });
        let o = &mut self.sets[s][i];
        (o.nodes, o.overridden) = (Vec::new(), false);
    }

    pub fn unload_level(&mut self, world: &mut dyn World) {
        for i in 0..self.sets[1].len() {
            self.clear(1, i, world);
        }
        self.sets[1].clear();
        self.flags.clear_temporary();
    }

    pub fn objective(&self, name: &str) -> Option<&Objective> {
        find(&self.sets, fnv1(name.as_bytes())).map(|(s, i)| &self.sets[s][i])
    }

    pub fn waiting_at(&self, name: &str) -> Option<&Node> {
        let o = self.objective(name)?;
        o.nodes.get(o.cur?)
    }
}
