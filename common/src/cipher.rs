//! Defines the standard use of a symmetric cypher, using `AES256-GCM-SIV`.

use aes_gcm_siv::aead::{AeadInPlace, KeyInit};
use aes_gcm_siv::{Aes256GcmSiv, Nonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use serde_derive::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use sha2::Sha256;
use std::fmt::Debug;
use std::marker::PhantomData;
use zeroize::Zeroize;

use crate::Hash;

/// Domain-separation tag for HKDF-SHA256 used to derive the AES-256-GCM-SIV key from a
/// content hash. Changing this breaks wire compatibility.
const HKDF_INFO: &[u8] = b"samizdat-transfer-cipher v1";

/// A symmetric cypher for coding data using `AES256-GCM-SIV`.
#[derive(Clone)]
pub struct TransferCipher {
    /// A nonce for the cipher.
    nonce: Nonce,
    /// The underlying symmetric cypher implementation.
    cipher: Aes256GcmSiv,
}

impl TransferCipher {
    /// Create a new transfer cypher from a content hash and a nonce. The content hash
    /// is run through HKDF-SHA256 to derive a 32-byte AES-256 key. The supplied `nonce`
    /// provides the 12-byte AEAD nonce (AES-GCM-SIV is misuse-resistant, so reuse of
    /// `(content_hash, nonce)` does not catastrophically leak plaintexts).
    pub fn new(content_hash: &Hash, nonce: &Hash) -> TransferCipher {
        let hk = Hkdf::<Sha256>::new(None, &content_hash.0);
        let mut key = [0u8; 32];
        hk.expand(HKDF_INFO, &mut key)
            .expect("32 bytes is well within HKDF-SHA256's output limit");

        let cipher = Aes256GcmSiv::new_from_slice(&key).expect("slice has correct key size");
        key.zeroize();

        let nonce = *Nonce::from_slice(&nonce[..12]);

        TransferCipher { nonce, cipher }
    }

    /// Encrypts a piece of data in place. Returns an error only if the buffer is too
    /// large for AES-GCM-SIV to handle (effectively never for real-world payloads).
    pub fn encrypt(&self, buf: &mut Vec<u8>) -> Result<(), crate::Error> {
        self.cipher
            .encrypt_in_place(&self.nonce, b"", buf)
            .map_err(|e| format!("AEAD encrypt failed: {e}").into())
    }

    /// Decrypts a piece of authenticated ciphertext in place. Returns an error if the
    /// AEAD tag does not validate; callers MUST treat this as authentication failure
    /// and discard the buffer.
    pub fn decrypt(&self, buf: &mut Vec<u8>) -> Result<(), crate::Error> {
        self.cipher
            .decrypt_in_place(&self.nonce, b"", buf)
            .map_err(|e| format!("AEAD decrypt/auth failed: {e}").into())
    }

    /// Encrypts a serializable object, using bincode to generate the binary data.
    pub fn encrypt_message<T>(&self, message: &T) -> Encrypted<T>
    where
        T: Serialize + for<'a> Deserialize<'a>,
    {
        Encrypted::new(message, self)
    }

    /// Encrypts a serializable object, using bincode to generate the binary data. This
    /// method also erases the type information of the message.
    pub fn encrypt_opaque<T>(&self, message: &T) -> OpaqueEncrypted
    where
        T: Serialize + for<'a> Deserialize<'a>,
    {
        OpaqueEncrypted::new(message, self)
    }
}

/// An encrypted piece of information, with type information on the encrypted content.
#[derive(Debug, SerdeSerialize, SerdeDeserialize)]
pub struct Encrypted<T> {
    data: Vec<u8>,
    _phantom: PhantomData<T>,
}

impl<T> Clone for Encrypted<T> {
    fn clone(&self) -> Encrypted<T> {
        Encrypted {
            data: self.data.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T> Encrypted<T>
where
    T: Serialize + for<'a> Deserialize<'a>,
{
    /// Encrypts a piece of information using a [`TransferCipher`].
    fn new(thing: &T, cipher: &TransferCipher) -> Encrypted<T> {
        let mut data = bincode::serialize(thing).expect("can serialize");
        cipher
            .encrypt(&mut data)
            .expect("AEAD encrypt cannot fail on bounded buffers");
        Encrypted {
            data,
            _phantom: PhantomData,
        }
    }

    /// Decrypts the encrypted data using a supplied [`TransferCipher`]. If the
    /// [`TransferCipher`] does not correspond to the original cipher, this method will
    /// fail with and error.
    pub fn decrypt_with(mut self, cipher: &TransferCipher) -> Result<T, crate::Error> {
        cipher.decrypt(&mut self.data)?;
        Ok(bincode::deserialize(&self.data)?)
    }
}

/// An encrypted piece of information, with type information on the encrypted content
/// erased.
#[derive(Clone, SerdeSerialize, SerdeDeserialize)]
pub struct OpaqueEncrypted {
    data: Vec<u8>,
}

impl Debug for OpaqueEncrypted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", base64_url::encode(&self.data))
    }
}

impl OpaqueEncrypted {
    /// Encrypts a piece of information using a [`TransferCipher`].
    pub fn new<T>(thing: &T, cipher: &TransferCipher) -> OpaqueEncrypted
    where
        T: Serialize,
    {
        let mut data = bincode::serialize(thing).expect("can serialize");
        cipher
            .encrypt(&mut data)
            .expect("AEAD encrypt cannot fail on bounded buffers");
        OpaqueEncrypted { data }
    }

    /// Decrypts the encrypted data using a supplied [`TransferCipher`] and the expected
    /// data type of the output. If the [`TransferCipher`] does not correspond to the
    /// original cipher or the data type does not match the original type, this method will
    /// fail with and error.
    pub fn decrypt_with<T>(mut self, cipher: &TransferCipher) -> Result<T, crate::Error>
    where
        T: for<'a> Deserialize<'a>,
    {
        cipher.decrypt(&mut self.data)?;
        Ok(bincode::deserialize(&self.data)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for P6; derivation uses HKDF; this just locks in round-tripping.
    #[test]
    fn round_trip() {
        let cipher = TransferCipher::new(&Hash::rand(), &Hash::rand());
        let mut buf = b"hello samizdat".to_vec();
        let plain = buf.clone();
        cipher.encrypt(&mut buf).unwrap();
        assert_ne!(buf, plain, "ciphertext should differ from plaintext");
        cipher.decrypt(&mut buf).unwrap();
        assert_eq!(buf, plain);
    }

    /// Regression test for P2; tampered ciphertext used to be silently accepted because
    /// `.ok()` discarded the AEAD failure and parsing proceeded over the intermediate
    /// buffer.
    #[test]
    fn decrypt_rejects_tampered_ciphertext() {
        let cipher = TransferCipher::new(&Hash::rand(), &Hash::rand());
        let mut buf = b"top secret payload".to_vec();
        cipher.encrypt(&mut buf).unwrap();

        // Flip a bit somewhere in the middle.
        buf[3] ^= 0x01;

        assert!(
            cipher.decrypt(&mut buf).is_err(),
            "AEAD authentication failure must surface"
        );
    }

    /// Regression test for P2; wrong key must produce an error, not garbage.
    #[test]
    fn decrypt_rejects_wrong_key() {
        let nonce = Hash::rand();
        let alice = TransferCipher::new(&Hash::rand(), &nonce);
        let mallory = TransferCipher::new(&Hash::rand(), &nonce);

        let mut buf = b"only alice can read this".to_vec();
        alice.encrypt(&mut buf).unwrap();

        assert!(mallory.decrypt(&mut buf).is_err());
    }

    /// Regression test for P2; Encrypted<T>::decrypt_with must propagate AEAD failure
    /// rather than feed corrupted bytes into bincode.
    #[test]
    fn encrypted_decrypt_with_wrong_key_fails() {
        let nonce = Hash::rand();
        let alice = TransferCipher::new(&Hash::rand(), &nonce);
        let mallory = TransferCipher::new(&Hash::rand(), &nonce);

        let payload = ("answer".to_string(), 42u32);
        let blob = alice.encrypt_message(&payload);
        let result: Result<(String, u32), _> = blob.decrypt_with(&mallory);
        assert!(result.is_err());
    }

    #[test]
    fn opaque_encrypted_decrypt_with_wrong_key_fails() {
        let nonce = Hash::rand();
        let alice = TransferCipher::new(&Hash::rand(), &nonce);
        let mallory = TransferCipher::new(&Hash::rand(), &nonce);

        let blob = alice.encrypt_opaque(&vec![1u32, 2, 3]);
        let result: Result<Vec<u32>, _> = blob.decrypt_with(&mallory);
        assert!(result.is_err());
    }
}
