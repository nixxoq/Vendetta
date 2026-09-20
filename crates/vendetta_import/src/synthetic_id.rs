use sha2::{Digest, Sha256};
use vendetta_model::{PeerId, PeerType};

pub const SYNTHETIC_CHAT_ID_BASE: i64 = -8_000_000_000_000_000_000;
pub const SYNTHETIC_SENDER_ID_BASE: i64 = -6_000_000_000_000_000_000;
pub const SYNTHETIC_MODULO: i64 = 1_000_000_000_000_000;

pub fn generate_synthetic_chat_id(
    title: &str,
    peer_type: PeerType,
    discriminator: Option<&str>,
) -> PeerId {
    let disc_bytes = discriminator.map(|d| d.trim().as_bytes());
    let parts: [&[u8]; 6] = [
        b"vendetta:synthetic:chat:",
        title.trim().as_bytes(),
        b":",
        peer_type.as_ref().as_bytes(),
        b":",
        disc_bytes.unwrap_or_default(),
    ];

    hash_parts_to_synthetic_id(SYNTHETIC_CHAT_ID_BASE, &parts)
}

pub fn generate_synthetic_sender_id(
    chat_id: PeerId,
    sender_name: &str,
    userpic_marker: Option<&str>,
) -> PeerId {
    let chat_id_bytes = chat_id.raw().to_be_bytes();
    let marker_bytes = userpic_marker.map(|m| m.trim().as_bytes());
    let parts: [&[u8]; 6] = [
        b"vendetta:synthetic:sender:",
        &chat_id_bytes,
        b":",
        sender_name.trim().as_bytes(),
        b":",
        marker_bytes.unwrap_or_default(),
    ];

    hash_parts_to_synthetic_id(SYNTHETIC_SENDER_ID_BASE, &parts)
}

fn hash_parts_to_synthetic_id(base: i64, parts: &[&[u8]]) -> PeerId {
    let hasher = parts.iter().fold(Sha256::new(), |mut acc, part| {
        acc.update(part);
        acc
    });

    let hash = hasher.finalize();
    let num = u64::from_be_bytes(
        hash.get(..8)
            .and_then(|s| s.try_into().ok())
            .unwrap_or_default(),
    );
    let offset = (num % (SYNTHETIC_MODULO as u64)) as i64;
    PeerId::new(base - offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synth_1() {
        let chat_id_1 = generate_synthetic_chat_id("Test Chat", PeerType::User, None);
        let chat_id_2 = generate_synthetic_chat_id("Test Chat", PeerType::User, None);
        assert_eq!(chat_id_1, chat_id_2);

        assert!(chat_id_1.raw() <= -8_000_000_000_000_000_000);
        assert!(chat_id_1.raw() > -9_000_000_000_000_000_000);

        let sender_id_1 = generate_synthetic_sender_id(chat_id_1, "Alice", Some("userpic5"));
        let sender_id_2 = generate_synthetic_sender_id(chat_id_1, "Alice", Some("userpic5"));
        assert_eq!(sender_id_1, sender_id_2);

        assert!(sender_id_1.raw() <= -6_000_000_000_000_000_000);
        assert!(sender_id_1.raw() > -7_000_000_000_000_000_000);
    }

    #[test]
    fn synth_2_dist() {
        let chat_a = generate_synthetic_chat_id("Chat A", PeerType::User, None);
        let chat_b = generate_synthetic_chat_id("Chat B", PeerType::User, None);
        assert_ne!(chat_a, chat_b);

        let sender_alice = generate_synthetic_sender_id(chat_a, "Alice", Some("A"));
        let sender_bob = generate_synthetic_sender_id(chat_a, "Bob", Some("B"));
        assert_ne!(sender_alice, sender_bob);
    }
}
