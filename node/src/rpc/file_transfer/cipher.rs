use aes_gcm_siv::aead::{AeadInPlace, NewAead};
use aes_gcm_siv::{Aes256GcmSiv, Key, Nonce};

use samizdat_common::Hash;

pub struct TransferCipher {
    nonce: Nonce,
    cipher: Aes256GcmSiv,
}

impl TransferCipher {
    pub fn new(content_hash: &Hash, nonce: &Hash) -> TransferCipher {
        fn extend(a: &[u8; 28]) -> [u8; 32] {
            let mut ext = [0; 32];
            for (i, byte) in a.iter().enumerate() {
                ext[i] = *byte;
            }

            ext
        }

        let hash_ext = extend(&content_hash.0);
        let key = Key::from_slice(&hash_ext);
        let cipher = Aes256GcmSiv::new(&key);

        let nonce = *Nonce::from_slice(&nonce[..12]);

        TransferCipher { cipher, nonce }
    }

    pub fn encrypt(&self, buf: &mut Vec<u8>) {
        self.cipher.encrypt_in_place(&self.nonce, b"", buf).ok();
    }

    pub fn decrypt(&self, buf: &mut Vec<u8>) {
        self.cipher.decrypt_in_place(&self.nonce, b"", buf).ok();
    }
}
