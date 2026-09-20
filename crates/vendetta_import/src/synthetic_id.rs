use sha2::{Digest, Sha256};
use vendetta_model::{PeerId, PeerType};

pub const SYNTHETIC_CHAT_ID_BASE: i64 = -8_000_000_000_000_000_000;
pub const SYNTHETIC_SENDER_ID_BASE: i64 = -6_000_000_000_000_000_000;
pub const SYNTHETIC_MODULO: i64 = 1_000_000_000_000_000;

/// Generate a deterministic synthetic `PeerId` for an imported chat without native MTProto ID.
///
/// Identity is derived exclusively from logical chat metadata (normalized title, peer type,
/// and optional internal export discriminator such as a relative subfolder token).
/// Does NOT couple to host filesystem paths.
pub fn generate_synthetic_chat_id(
    title: &str,
    peer_type: PeerType,
    discriminator: Option<&str>,
) -> PeerId {
    let mut hasher = Sha256::new();
    hasher.update(b"vendetta:synthetic:chat:");
    hasher.update(title.trim().as_bytes());
    hasher.update(b":");
    hasher.update(peer_type.as_ref().as_bytes());
    hasher.update(b":");
    if let Some(disc) = discriminator {
        hasher.update(disc.trim().as_bytes());
    }

    let hash = hasher.finalize();
    let num = u64::from_be_bytes(hash[0..8].try_into().unwrap_or_default());
    let offset = (num % (SYNTHETIC_MODULO as u64)) as i64;
    PeerId::new(SYNTHETIC_CHAT_ID_BASE - offset)
}

/// Generate a deterministic synthetic `PeerId` for an imported sender without native MTProto ID.
///
/// Scoped to the parent chat ID and strongest available stable sender markers
/// (e.g. sender name and userpic initials or style class).
pub fn generate_synthetic_sender_id(
    chat_id: PeerId,
    sender_name: &str,
    userpic_marker: Option<&str>,
) -> PeerId {
    let mut hasher = Sha256::new();
    hasher.update(b"vendetta:synthetic:sender:");
    hasher.update(chat_id.raw().to_be_bytes());
    hasher.update(b":");
    hasher.update(sender_name.trim().as_bytes());
    hasher.update(b":");
    if let Some(marker) = userpic_marker {
        hasher.update(marker.trim().as_bytes());
    }

    let hash = hasher.finalize();
    let num = u64::from_be_bytes(hash[0..8].try_into().unwrap_or_default());
    let offset = (num % (SYNTHETIC_MODULO as u64)) as i64;
    PeerId::new(SYNTHETIC_SENDER_ID_BASE - offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_ids_are_deterministic_and_within_namespace() {
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
    fn distinct_synthetic_fixture_identities_produce_distinct_ids() {
        let chat_a = generate_synthetic_chat_id("Chat A", PeerType::User, None);
        let chat_b = generate_synthetic_chat_id("Chat B", PeerType::User, None);
        assert_ne!(chat_a, chat_b);

        let sender_alice = generate_synthetic_sender_id(chat_a, "Alice", Some("A"));
        let sender_bob = generate_synthetic_sender_id(chat_a, "Bob", Some("B"));
        assert_ne!(sender_alice, sender_bob);
    }
}
