use ssx_rules::results::{ResultsFlow, RunEnd, WorldTourRule, bots_place, explore_outcome};

use super::{EventResult, EventSetup, EventType, Medal};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ranking {
    pub rivals: Vec<u32>,
    pub rank: Option<usize>,
    pub medal: Option<Medal>,
    pub place: u32,
}

impl EventSetup {
    pub(super) fn flow_of(def: Option<&super::EventDef>) -> Option<ResultsFlow> {
        def.and_then(|d| ResultsFlow::of_drop_type(d.message.kind))
    }

    pub fn world_tour_rule(&self) -> WorldTourRule {
        let event_type = self.event_type.map_or(9, |t| t as i64);
        WorldTourRule { event_type, places: self.places, goal: self.goal }
    }

    pub fn rank(&self, value: u32, failed: bool, racers: Vec<u32>) -> Ranking {
        let place = bots_place(self.event_type, value, &racers);
        match self.results {
            Some(ResultsFlow::Explore) => {
                let (rank, _) = explore_outcome(self.event_type, failed, value, &self.rivals);
                Ranking { rivals: self.rivals.clone(), rank, medal: rank.and_then(Medal::from_rank), place }
            }
            Some(ResultsFlow::WorldTour) => {
                let bots = matches!(self.event_type, Some(EventType::RaceBots | EventType::TrickBots));
                let at = if place == u32::MAX { racers.len() } else { place as usize };
                Ranking { rank: (bots && !failed).then_some(at), rivals: racers, medal: None, place }
            }
            None => Ranking { rivals: racers, rank: None, medal: None, place },
        }
    }
}

impl EventResult {
    pub fn run_end(&self, setup: &EventSetup, bot_scores: &[u32]) -> Option<RunEnd> {
        match setup.results? {
            ResultsFlow::Explore => {
                let (rank, won) = explore_outcome(self.event_type, self.failed, self.value, &setup.rivals);
                Some(RunEnd::explore(self.failed, self.value, rank, won))
            }
            ResultsFlow::WorldTour => {
                let won = setup.world_tour_rule().won(self.value, self.place, self.failed, bot_scores);
                Some(RunEnd::world_tour(self.value, self.place, won, self.failed))
            }
        }
    }
}
