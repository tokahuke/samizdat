//! Defines the standard use of a symmetric cypher, using `AES256-GCM-SIV`.

use aes_gcm_siv::aead::{AeadInPlace, KeyInit};
use aes_gcm_siv::{Aes256GcmSiv, Nonce};
use serde::{Deserialize, Serialize};
use serde_derive::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use std::fmt::Debug;
use std::marker::PhantomData;

use crate::Hash;

/// A symmetric cypher for coding data using `AES256-GCM-SIV`.
pub struct TransferCipher {
    /// A nonce for the cipher.
    nonce: Nonce,
    /// The underlying symmetric cypher implementation.
    cipher: Aes256GcmSiv,
}

impl TransferCipher {
    /// Create a new transfer cypher based on a given content hash and a nonce. The content
    /// hash works as the symmetric key.
    pub fn new(content_hash: &Hash, nonce: &Hash) -> TransferCipher {
        fn extend(a: &[u8; 28]) -> [u8; 32] {
            let mut ext = [0; 32];
            for (i, byte) in a.iter().enumerate() {
                ext[i] = *byte;
            }

            ext
        }

        let hash_ext = extend(&content_hash.0);
        let cipher = Aes256GcmSiv::new_from_slice(&hash_ext).expect("slice has correct key size");

        let nonce = *Nonce::from_slice(&nonce[..12]);

        TransferCipher { cipher, nonce }
    }

    /// Encrypts a piece of data, using the same container to hold the encrypted content.
    pub fn encrypt(&self, buf: &mut Vec<u8>) {
        self.cipher.encrypt_in_place(&self.nonce, b"", buf).ok();
    }

    /// Decrypts a piece of encrypted data, using the same container to hold the decrypted
    /// content.
    pub fn decrypt(&self, buf: &mut Vec<u8>) {
        self.cipher.decrypt_in_place(&self.nonce, b"", buf).ok();
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
        cipher.encrypt(&mut data);
        Encrypted {
            data,
            _phantom: PhantomData,
        }
    }

    /// Decrypts the encrypted data using a supplied [`TransferCipher`]. If the
    /// [`TransferCipher`] does not correspond to the original cipher, this method will
    /// fail with and error.
    pub fn decrypt_with(mut self, cipher: &TransferCipher) -> Result<T, crate::Error> {
        cipher.decrypt(&mut self.data);
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
        cipher.encrypt(&mut data);
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
        cipher.decrypt(&mut self.data);
        Ok(bincode::deserialize(&self.data)?)
    }
}
