//! Original offload-engine mailbox layout: exactly 132 right-justified words.
//! These are harness RAM conventions, not physical DMA addresses or a card ABI.
use crate::{Pcm24, FRAME_COUNT};

pub const AUDIO_WORDS: usize = FRAME_COUNT * 2;
pub const PACKET_WORDS: usize = AUDIO_WORDS + 4;
pub const STOP_SEQUENCE: u32 = 0x00ff_ffff;
pub const OP_CAPTURE: u32 = 2;
pub const OP_RETURN: u32 = 3;
pub type Packet = [u32; PACKET_WORDS];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub epoch: u32,
    pub sequence: u32,
}
impl Ticket {
    pub fn new(epoch: u32, sequence: u32) -> Result<Self, &'static str> {
        if epoch == 0 || epoch > STOP_SEQUENCE {
            return Err("epoch must be a nonzero 24-bit word");
        }
        if sequence >= STOP_SEQUENCE {
            return Err("sequence requires a fresh epoch");
        }
        Ok(Self { epoch, sequence })
    }
    pub fn capture(self) -> Packet {
        let mut packet = [0; PACKET_WORDS];
        packet[..4].copy_from_slice(&[OP_CAPTURE, self.epoch, self.sequence, AUDIO_WORDS as u32]);
        packet
    }
    pub fn returned(self, samples: &[Pcm24]) -> Result<Packet, &'static str> {
        if samples.len() != AUDIO_WORDS {
            return Err("return must contain exactly 128 audio words");
        }
        let mut packet = self.capture();
        packet[0] = OP_RETURN;
        for (destination, sample) in packet[4..].iter_mut().zip(samples) {
            *destination = sample.word();
        }
        Ok(packet)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureResponse {
    pub epoch: u32,
    pub current_block: u32,
    pub frame: u32,
    pub samples: [Pcm24; AUDIO_WORDS],
}
impl CaptureResponse {
    /// `None` is a valid firmware rejection (status 0), not a successful capture.
    /// `current_block` reports the engine's position; it is not a request echo.
    pub fn parse(words: &[u32], expected_epoch: u32) -> Result<Option<Self>, &'static str> {
        if words.len() != PACKET_WORDS || words.iter().any(|word| *word > STOP_SEQUENCE) {
            return Err("incomplete or non-24-bit response");
        }
        if words[1] != expected_epoch || expected_epoch == 0 || expected_epoch > STOP_SEQUENCE {
            return Err("stale response epoch");
        }
        if words[0] == 0 {
            return Ok(None);
        }
        if words[0] != 1 || words[3] >= FRAME_COUNT as u32 {
            return Err("invalid response status or frame");
        }
        Ok(Some(Self {
            epoch: words[1],
            current_block: words[2],
            frame: words[3],
            samples: std::array::from_fn(|index| Pcm24::from_word(words[index + 4])),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_preserve_every_signed_pcm_word_and_reject_partial_returns() {
        let audio = crate::fixture(AUDIO_WORDS);
        let ticket = Ticket::new(7, 0x0080_0000).unwrap();
        let capture = ticket.capture();
        assert_eq!(&capture[..4], &[2, 7, 0x0080_0000, 128]);
        assert!(capture[4..].iter().all(|word| *word == 0));
        let returned = ticket.returned(&audio).unwrap();
        assert_eq!(&returned[..4], &[3, 7, 0x0080_0000, 128]);
        assert_eq!(
            &returned[4..],
            audio.iter().map(|sample| sample.word()).collect::<Vec<_>>()
        );
        assert!(ticket.returned(&audio[..127]).is_err());
        assert!(Ticket::new(0, 0).is_err());
        assert!(Ticket::new(STOP_SEQUENCE + 1, 0).is_err());
        assert!(Ticket::new(1, STOP_SEQUENCE).is_err());
        assert!(Ticket::new(1, STOP_SEQUENCE - 1).is_ok());
    }

    #[test]
    fn response_validates_epoch_status_frame_and_full_descriptor() {
        let mut words = [0; PACKET_WORDS];
        words[..4].copy_from_slice(&[1, 11, 17, 63]);
        words[4..].copy_from_slice(
            &crate::fixture(AUDIO_WORDS)
                .iter()
                .map(|x| x.word())
                .collect::<Vec<_>>(),
        );
        let response = CaptureResponse::parse(&words, 11).unwrap().unwrap();
        assert_eq!(
            (response.epoch, response.current_block, response.frame),
            (11, 17, 63)
        );
        assert_eq!(response.samples.as_slice(), crate::fixture(AUDIO_WORDS));
        assert!(CaptureResponse::parse(&words[..131], 11).is_err());
        assert!(CaptureResponse::parse(&words, 12).is_err());
        words[0] = 0;
        assert!(CaptureResponse::parse(&words, 11).unwrap().is_none());
        words[0] = 2;
        assert!(CaptureResponse::parse(&words, 11).is_err());
        words[0] = 1;
        words[3] = 64;
        assert!(CaptureResponse::parse(&words, 11).is_err());
        words[3] = 0;
        words[8] = 0x0100_0000;
        assert!(CaptureResponse::parse(&words, 11).is_err());
    }
}
