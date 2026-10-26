//! `og-common`: OpenGridder core definitions, network wire protocol, and utilities.
//! Zero-overhead pure-Rust implementation, MIT licensed.

pub mod audio_ring_buffer;
pub mod channel_mapper;
pub mod channel_set;
pub mod defaults;
pub mod image_diff;
pub mod key_and_mouse;
pub mod mdns;
pub mod message;
pub mod metrics;
pub mod server_plugin;
pub mod utils;
pub mod version;

#[cfg(test)]
mod tests {
    use crate::audio_ring_buffer::AudioRingBuffer;
    use crate::channel_mapper::ChannelMapper;
    use crate::channel_set::ChannelSet;
    use crate::image_diff::{apply_delta, get_delta};
    use crate::key_and_mouse::{get_key_code, get_key_name};
    use crate::message::{
        AudioMessage, HandshakeRequest, HandshakeResponse, MessageType, MidiEvent, PositionInfo,
        RawMessage, AG_PROTOCOL_VERSION,
    };
    use crate::server_plugin::ServerPlugin;
    use crate::utils::ServerInfo;
    use std::io::Cursor;

    #[test]
    fn test_audio_ring_buffer_bypass_delay() {
        let mut rb = AudioRingBuffer::<f32>::with_capacity(2, 8, true);
        rb.set_write_offset(4);
        let mut block = vec![vec![1.0, 2.0, 3.0, 4.0], vec![10.0, 20.0, 30.0, 40.0]];
        rb.process(&mut block, 4);
        // First 4 samples coming out should be the initial zeros (latency = 4 samples)
        assert_eq!(block[0], vec![0.0, 0.0, 0.0, 0.0]);
        let mut block2 = vec![vec![5.0, 6.0, 7.0, 8.0], vec![50.0, 60.0, 70.0, 80.0]];
        rb.process(&mut block2, 4);
        assert_eq!(block2[0], vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(block2[1], vec![10.0, 20.0, 30.0, 40.0]);
    }

    #[test]
    fn test_channel_set_and_mapper() {
        let mut cs = ChannelSet::with_counts(0, 4, 4);
        cs.set_input_active(0, true);
        cs.set_input_active(1, true);
        cs.set_output_active(0, true);
        cs.set_output_active(1, true);
        assert_eq!(cs.get_num_active_channels(true), 2);
        assert_eq!(cs.get_num_active_channels(false), 2);

        let mapper = ChannelMapper::with_mapping(&cs, true);
        let src = vec![
            vec![1.0f32, 1.0],
            vec![2.0f32, 2.0],
            vec![3.0f32, 3.0],
            vec![4.0f32, 4.0],
        ];
        let mut reduced = vec![vec![0.0f32; 2]; 2];
        mapper.map(&src, &mut reduced);
        assert_eq!(reduced[0], vec![1.0, 1.0]);
        assert_eq!(reduced[1], vec![2.0, 2.0]);
    }

    #[test]
    fn test_handshake_and_oversampling_messages() {
        let mut req = HandshakeRequest {
            version: AG_PROTOCOL_VERSION,
            channels_in: 2,
            channels_out: 2,
            channels_sc: 0,
            sample_rate: 48000.0,
            samples_per_block: 256,
            double_precision: false,
            client_id: 0x1234_5678,
            flags: 0,
            active_channels: 0b11,
        };
        req.set_flag(1);
        let bytes = req.to_bytes();
        let decoded = HandshakeRequest::from_bytes(&bytes).unwrap();
        assert_eq!(req, decoded);

        let mut resp = HandshakeResponse {
            version: AG_PROTOCOL_VERSION,
            flags: 0,
            port: 55099,
            unused: [0; 6],
        };
        resp.set_flag(2);
        let rbytes = resp.to_bytes();
        let rdecoded = HandshakeResponse::from_bytes(&rbytes).unwrap();
        assert_eq!(resp, rdecoded);

        // Verify SetPluginOversampling (Type 75 from marcosfork-1.2.1)
        let msg = RawMessage::set_plugin_oversampling(3, 4);
        assert_eq!(msg.msg_type, MessageType::SetPluginOversampling as i32);
        let mut wire = Vec::new();
        msg.send(&mut wire).unwrap();
        let mut cursor = Cursor::new(wire);
        let rx = RawMessage::read(&mut cursor).unwrap();
        assert_eq!(rx.parse_oversampling(), (3, 4));
    }

    #[test]
    fn test_audio_message_roundtrip() {
        let mut sender = AudioMessage::default();
        let audio_in = vec![vec![0.25f32, -0.5, 0.75, 1.0], vec![-0.25f32, 0.5, -0.75, -1.0]];
        let midi_in = vec![MidiEvent {
            sample_number: 2,
            data: vec![0x90, 60, 100],
        }];
        let pos = PositionInfo {
            bpm: 128.0,
            is_playing: true,
            ..Default::default()
        };

        let mut stream = Vec::new();
        sender
            .send_to_server_f32(&mut stream, &audio_in, &midi_in, &pos, 2, 4)
            .unwrap();

        let mut receiver = AudioMessage::default();
        let mut buf_f = Vec::new();
        let mut buf_d = Vec::new();
        let mut midi_out = Vec::new();
        let mut pos_out = PositionInfo::default();
        let mut cur = Cursor::new(stream);
        receiver
            .read_from_client(&mut cur, &mut buf_f, &mut buf_d, &mut midi_out, &mut pos_out)
            .unwrap();

        assert_eq!(buf_f, audio_in);
        assert_eq!(midi_out, midi_in);
        assert_eq!(pos_out.bpm, 128.0);
        assert!(pos_out.is_playing);
    }

    #[test]
    fn test_server_plugin_and_server_info() {
        let sp = ServerPlugin::new(
            "HybridEQ",
            "MistaMin",
            "VST3-1234",
            "VST3-1234",
            "VST3",
            "EQ",
            false,
            vec!["Stereo".to_string()],
        );
        let s = sp.to_string_serialized();
        let parsed = ServerPlugin::from_string(&s).expect("should parse");
        assert_eq!(sp, parsed);

        let srv = ServerInfo::new("127.0.0.1", "StudioMac", false, 0, "uuid-1", 0.25, true, "1.2.1");
        let ser = srv.serialize();
        let srv2 = ServerInfo::parse(&ser);
        assert!(srv.matches(&srv2));
    }

    #[test]
    fn test_image_diff_and_key_map() {
        let from = vec![10, 20, 30, 255, 50, 60, 70, 255];
        let to = vec![10, 20, 30, 255, 90, 91, 92, 255];
        let mut delta = vec![0u8; 8];
        assert_eq!(get_delta(&from, &to, &mut delta, 2, 1), 1);
        let mut dst = from.clone();
        assert_eq!(apply_delta(&mut dst, &delta, 2, 1), 1);
        assert_eq!(dst, to);

        assert_eq!(get_key_name(get_key_code("Space")), "Space");
    }
}
