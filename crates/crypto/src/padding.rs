//! Padding, so that the length of a sealed message says little about what is in it.
//!
//! A padded message is a four-byte length, the message, then zeros up to the next size
//! step: 256 bytes, then doubling. Every short text message is the same size on the
//! wire; a server or eavesdropper learns only which step a message falls in.

const SMALLEST: usize = 256;
const LARGEST: usize = 32 * 1024;

/// The longest message that can be sent. Sealed, it still fits the server's limit.
pub const MAX_PLAINTEXT_BYTES: usize = LARGEST - 4;

pub fn pad(plaintext: &[u8]) -> Option<Vec<u8>> {
    if plaintext.len() > MAX_PLAINTEXT_BYTES {
        return None;
    }
    let size = (plaintext.len() + 4).next_power_of_two().max(SMALLEST);
    let mut padded = Vec::with_capacity(size);
    padded.extend((plaintext.len() as u32).to_be_bytes());
    padded.extend(plaintext);
    padded.resize(size, 0);
    Some(padded)
}

pub fn unpad(padded: &[u8]) -> Option<Vec<u8>> {
    let (length, rest) = padded.split_first_chunk::<4>()?;
    rest.get(..u32::from_be_bytes(*length) as usize).map(<[u8]>::to_vec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_round_trips_and_lands_on_a_size_step() {
        for length in [0, 1, 251, 252, 253, 1000, 5000, MAX_PLAINTEXT_BYTES] {
            let message = vec![0xA5u8; length];
            let padded = pad(&message).unwrap();
            assert!(padded.len().is_power_of_two() && padded.len() >= SMALLEST, "{length}");
            assert!(padded.len() >= length + 4);
            assert_eq!(unpad(&padded).unwrap(), message, "{length}");
        }
    }

    #[test]
    fn short_messages_are_all_the_same_size() {
        assert_eq!(pad(b"ok").unwrap().len(), pad(&[b'x'; 252]).unwrap().len());
        assert_eq!(pad(&[b'x'; 252]).unwrap().len(), 256);
        assert_eq!(pad(&[b'x'; 253]).unwrap().len(), 512);
    }

    #[test]
    fn oversized_and_malformed_input_is_refused() {
        assert!(pad(&vec![0u8; MAX_PLAINTEXT_BYTES + 1]).is_none());
        assert!(unpad(&[0, 0]).is_none());
        assert!(unpad(&[0, 0, 0, 9, 1, 2]).is_none(), "claims more bytes than there are");
    }
}
