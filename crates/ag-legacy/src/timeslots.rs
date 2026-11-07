// Adapted from the archived MIT OpenGridder-TDM-Emulator. See legacy/PROVENANCE.md.
use crate::pcm::Pcm24;

/// Configurable frame-major timeslot fabric. All slot numbers are abstract indices,
/// never hardware registers. Unrouted output slots are silence.
#[derive(Clone, Debug)]
pub struct Timeslots {
    slots: usize,
    routes: Vec<usize>,
}
impl Timeslots {
    pub fn new(slots: usize, routes: Vec<usize>) -> Result<Self, &'static str> {
        if slots == 0 || slots > 1024 || routes.is_empty() || routes.len() > slots {
            return Err("invalid timeslot geometry");
        }
        let mut seen = std::collections::BTreeSet::new();
        if routes.iter().any(|&s| s >= slots || !seen.insert(s)) {
            return Err("slot out of range or duplicated");
        }
        Ok(Self { slots, routes })
    }
    pub fn channels(&self) -> usize {
        self.routes.len()
    }
    pub fn capture(&self, fabric: &[Pcm24]) -> Result<Vec<Pcm24>, &'static str> {
        if fabric.is_empty() || !fabric.len().is_multiple_of(self.slots) {
            return Err("partial or empty timeslot frame");
        }
        Ok(fabric
            .chunks_exact(self.slots)
            .flat_map(|frame| self.routes.iter().map(move |&s| frame[s]))
            .collect())
    }
    pub fn render(&self, audio: &[Pcm24]) -> Result<Vec<Pcm24>, &'static str> {
        if audio.is_empty() || !audio.len().is_multiple_of(self.channels()) {
            return Err("partial or empty audio frame");
        }
        let mut fabric = vec![Pcm24::ZERO; audio.len() / self.channels() * self.slots];
        for (frame, samples) in fabric
            .chunks_exact_mut(self.slots)
            .zip(audio.chunks_exact(self.channels()))
        {
            for (&slot, &sample) in self.routes.iter().zip(samples) {
                frame[slot] = sample;
            }
        }
        Ok(fabric)
    }
}
