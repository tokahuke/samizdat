use aes_gcm_siv::aead::{AeadInPlace, NewAead};
use aes_gcm_siv::{Aes256GcmSiv, Key, Nonce};
use serde::{Deserialize, Serialize};
use serde_derive::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use std::fmt::Debug;
use std::marker::PhantomData;

use crate::Hash;

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
        let cipher = Aes256GcmSiv::new(key);

        let nonce = *Nonce::from_slice(&nonce[..12]);

        TransferCipher { cipher, nonce }
    }

    pub fn encrypt(&self, buf: &mut Vec<u8>) {
        self.cipher.encrypt_in_place(&self.nonce, b"", buf).ok();
    }

    pub fn decrypt(&self, buf: &mut Vec<u8>) {
        self.cipher.decrypt_in_place(&self.nonce, b"", buf).ok();
    }

    pub fn encrypt_message<T>(&self, message: &T) -> Encrypted<T>
    where
        T: Serialize + for<'a> Deserialize<'a>,
    {
        Encrypted::new(message, self)
    }

    pub fn encrypt_opaque<T>(&self, message: &T) -> OpaqueEncrypted
    where
        T: Serialize + for<'a> Deserialize<'a>,
    {
        OpaqueEncrypted::new(message, self)
    }
}

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
    fn new(thing: &T, cipher: &TransferCipher) -> Encrypted<T> {
        let mut data = bincode::serialize(thing).expect("can serialize");
        cipher.encrypt(&mut data);
        Encrypted {
            data,
            _phantom: PhantomData,
        }
    }

    pub fn decrypt_with(mut self, cipher: &TransferCipher) -> Result<T, crate::Error> {
        cipher.decrypt(&mut self.data);
        Ok(bincode::deserialize(&self.data)?)
    }
}

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
    pub fn new<T>(thing: &T, cipher: &TransferCipher) -> OpaqueEncrypted
    where
        T: Serialize,
    {
        let mut data = bincode::serialize(thing).expect("can serialize");
        cipher.encrypt(&mut data);
        OpaqueEncrypted { data }
    }

    pub fn decrypt_with<T>(mut self, cipher: &TransferCipher) -> Result<T, crate::Error>
    where
        T: for<'a> Deserialize<'a>,
    {
        cipher.decrypt(&mut self.data);
        Ok(bincode::deserialize(&self.data)?)
    }
}
