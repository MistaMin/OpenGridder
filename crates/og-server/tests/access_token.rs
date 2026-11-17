//! The token check is a process-wide setting, so it lives in its own test binary.

use og_protocol::HandshakeRequest;
use og_server::{require_token, validate_handshake, RejectReason};

#[test]
fn only_the_servers_token_passes_the_handshake() {
    require_token(0x1234_5678_9ABC_DEF1);
    let with = |token| HandshakeRequest { token, ..HandshakeRequest::default() };
    assert_eq!(validate_handshake(&with(0)), Err(RejectReason::Token));
    assert_eq!(validate_handshake(&with(0x1234_5678_9ABC_DEF0)), Err(RejectReason::Token));
    assert_eq!(validate_handshake(&with(0x1234_5678_9ABC_DEF1)), Ok(()));
}
