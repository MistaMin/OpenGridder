// Adapted from the archived MIT OpenGridder-TDM-Emulator. See LEGACY.md; the source is now in the private legacy wrappers repository.
use crate::pcm::Pcm24;
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ticket {
    pub epoch: u64,
    pub sequence: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub ticket: Ticket,
    pub audio: Vec<Pcm24>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturnError {
    Stale,
    NotDispatched,
    Duplicate,
    WrongLength,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub overflows: u64,
    pub underruns: u64,
    pub rejected: u64,
    pub played: u64,
}
enum Pending {
    Queued(Vec<Pcm24>),
    InFlight,
    Ready(Vec<Pcm24>),
}

/// Single-threaded bounded behavioral DMA model; not lock-free, RT-safe, or a
/// shared-memory/card ABI. A playback call is an explicit block deadline.
/// Host returns can arrive in any order; only the matching epoch/sequence plays.
pub struct Transport {
    capacity: usize,
    words: usize,
    epoch: u64,
    next_submit: u64,
    next_play: u64,
    pending: BTreeMap<u64, Pending>,
    requests: VecDeque<u64>,
    stats: Stats,
}
impl Transport {
    pub fn new(capacity: usize, frames: usize, channels: usize) -> Result<Self, &'static str> {
        let words = frames.checked_mul(channels).ok_or("block size overflow")?;
        if capacity == 0 || capacity > 4096 || words == 0 || words > crate::MAX_WORDS {
            return Err("invalid transport geometry");
        }
        Ok(Self {
            capacity,
            words,
            epoch: 0,
            next_submit: 0,
            next_play: 0,
            pending: BTreeMap::new(),
            requests: VecDeque::new(),
            stats: Stats::default(),
        })
    }
    pub fn stats(&self) -> Stats {
        self.stats
    }
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
    /// Advances capture sequence even on overflow, preserving the missing deadline.
    pub fn submit(&mut self, audio: Vec<Pcm24>) -> Result<Ticket, &'static str> {
        if audio.len() != self.words {
            return Err("wrong capture length");
        }
        let sequence = self.next_submit;
        self.next_submit = sequence
            .checked_add(1)
            .ok_or("sequence exhausted; reset required")?;
        let ticket = Ticket {
            epoch: self.epoch,
            sequence,
        };
        if self.pending.len() == self.capacity {
            self.stats.overflows += 1;
            return Ok(ticket);
        }
        self.pending.insert(sequence, Pending::Queued(audio));
        self.requests.push_back(sequence);
        Ok(ticket)
    }
    /// Control-thread service, separate from deadline playback.
    pub fn take_request(&mut self) -> Option<Request> {
        while let Some(sequence) = self.requests.pop_front() {
            if let Some(pending) = self.pending.get_mut(&sequence) {
                let old = std::mem::replace(pending, Pending::InFlight);
                if let Pending::Queued(audio) = old {
                    return Some(Request {
                        ticket: Ticket {
                            epoch: self.epoch,
                            sequence,
                        },
                        audio,
                    });
                }
                *pending = old;
            }
        }
        None
    }
    pub fn complete(&mut self, ticket: Ticket, audio: Vec<Pcm24>) -> Result<(), ReturnError> {
        let error = if ticket.epoch != self.epoch || !self.pending.contains_key(&ticket.sequence) {
            Some(ReturnError::Stale)
        } else if audio.len() != self.words {
            Some(ReturnError::WrongLength)
        } else {
            match self.pending[&ticket.sequence] {
                Pending::Queued(_) => Some(ReturnError::NotDispatched),
                Pending::Ready(_) => Some(ReturnError::Duplicate),
                Pending::InFlight => None,
            }
        };
        if let Some(error) = error {
            self.stats.rejected += 1;
            return Err(error);
        }
        self.pending.insert(ticket.sequence, Pending::Ready(audio));
        Ok(())
    }
    /// Missing result becomes silence and is retired permanently, preventing
    /// late audio from leaking into a later block. No repeat-last-buffer fallback.
    pub fn play_next(&mut self) -> Result<Vec<Pcm24>, &'static str> {
        if self.next_play == self.next_submit {
            return Err("no captured block has a playback deadline");
        }
        let sequence = self.next_play;
        self.next_play += 1;
        // Bound the service queue even when no host service occurs.
        self.requests.retain(|&s| s > sequence);
        match self.pending.remove(&sequence) {
            Some(Pending::Ready(audio)) => {
                self.stats.played += 1;
                Ok(audio)
            }
            _ => {
                self.stats.underruns += 1;
                Ok(vec![Pcm24::ZERO; self.words])
            }
        }
    }
    /// A new epoch invalidates every old completion, including sequence zero.
    /// Statistics are cumulative across resets.
    pub fn reset(&mut self) -> Result<(), &'static str> {
        self.epoch = self.epoch.checked_add(1).ok_or("epoch exhausted")?;
        self.next_submit = 0;
        self.next_play = 0;
        self.pending.clear();
        self.requests.clear();
        Ok(())
    }
}
