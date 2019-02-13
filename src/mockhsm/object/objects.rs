use failure::Error;
use ring::aead::{self, Aad, Nonce, OpeningKey, SealingKey, AES_128_GCM, AES_256_GCM};
use std::collections::{btree_map::Iter as BTreeMapIter, BTreeMap};

use super::{
    Object, Payload, WrappedObject, DEFAULT_AUTHENTICATION_KEY_LABEL, WRAPPED_DATA_MAC_SIZE,
};
use crate::{
    authentication_key::{AuthenticationKey, AUTHENTICATION_KEY_SIZE},
    credentials::DEFAULT_AUTHENTICATION_KEY_ID,
    serialization::{deserialize, serialize},
    Algorithm, AuthenticationAlg, Capability, Domain, ObjectHandle, ObjectId, ObjectInfo,
    ObjectLabel, ObjectOrigin, ObjectType, WrapAlg, WrapNonce,
};

/// Objects stored in the `MockHsm`
#[derive(Debug)]
pub(crate) struct Objects(BTreeMap<ObjectHandle, Object>);

impl Default for Objects {
    fn default() -> Self {
        let mut objects = BTreeMap::new();

        // Insert default authentication key
        let authentication_key_handle =
            ObjectHandle::new(DEFAULT_AUTHENTICATION_KEY_ID, ObjectType::AuthenticationKey);

        let authentication_key_info = ObjectInfo {
            object_id: DEFAULT_AUTHENTICATION_KEY_ID,
            object_type: ObjectType::AuthenticationKey,
            algorithm: Algorithm::Auth(AuthenticationAlg::YUBICO_AES),
            capabilities: Capability::all(),
            delegated_capabilities: Capability::all(),
            domains: Domain::all(),
            length: AUTHENTICATION_KEY_SIZE as u16,
            sequence: 1,
            origin: ObjectOrigin::Imported,
            label: DEFAULT_AUTHENTICATION_KEY_LABEL.into(),
        };

        let authentication_key_payload = Payload::AuthenticationKey(AuthenticationKey::default());

        let _ = objects.insert(
            authentication_key_handle,
            Object {
                object_info: authentication_key_info,
                payload: authentication_key_payload,
            },
        );

        Objects(objects)
    }
}

impl Objects {
    /// Generate a new object in the MockHsm
    pub fn generate(
        &mut self,
        object_id: ObjectId,
        object_type: ObjectType,
        algorithm: Algorithm,
        label: ObjectLabel,
        capabilities: Capability,
        delegated_capabilities: Capability,
        domains: Domain,
    ) {
        let payload = Payload::generate(algorithm);
        let length = payload.len();

        let object_info = ObjectInfo {
            object_id,
            object_type,
            algorithm,
            capabilities,
            delegated_capabilities,
            domains,
            length,
            sequence: 1,
            origin: ObjectOrigin::Generated,
            label,
        };

        let handle = ObjectHandle::new(object_id, object_type);

        let object = Object {
            object_info,
            payload,
        };

        assert!(self.0.insert(handle, object).is_none());
    }

    /// Get an object
    pub fn get(&self, object_id: ObjectId, object_type: ObjectType) -> Option<&Object> {
        self.0.get(&ObjectHandle::new(object_id, object_type))
    }

    /// Put a new object in the MockHsm
    pub fn put(
        &mut self,
        object_id: ObjectId,
        object_type: ObjectType,
        algorithm: Algorithm,
        label: ObjectLabel,
        capabilities: Capability,
        delegated_capabilities: Capability,
        domains: Domain,
        data: &[u8],
    ) {
        let payload = Payload::new(algorithm, data);
        let length = payload.len();

        let object_info = ObjectInfo {
            object_id,
            object_type,
            algorithm,
            capabilities,
            delegated_capabilities,
            domains,
            length,
            sequence: 1,
            origin: ObjectOrigin::Imported,
            label,
        };

        let handle = ObjectHandle::new(object_id, object_type);

        let object = Object {
            object_info,
            payload,
        };

        assert!(self.0.insert(handle, object).is_none());
    }

    /// Remove an object
    pub fn remove(&mut self, object_id: ObjectId, object_type: ObjectType) -> Option<Object> {
        self.0.remove(&ObjectHandle::new(object_id, object_type))
    }

    /// Serialize an object as ciphertext
    pub fn wrap(
        &mut self,
        wrap_key_id: ObjectId,
        object_id: ObjectId,
        object_type: ObjectType,
        wrap_nonce: &WrapNonce,
    ) -> Result<Vec<u8>, Error> {
        let wrap_key = match self.get(wrap_key_id, ObjectType::WrapKey) {
            Some(k) => k,
            None => bail!("no such wrap key: {:?}", wrap_key_id),
        };

        let sealing_key = match wrap_key.algorithm().wrap().unwrap() {
            // TODO: actually use AES-CCM
            WrapAlg::AES128_CCM => SealingKey::new(&AES_128_GCM, wrap_key.payload.as_ref()),
            WrapAlg::AES256_CCM => SealingKey::new(&AES_256_GCM, wrap_key.payload.as_ref()),
            unsupported => bail!("unsupported wrap key algorithm: {:?}", unsupported),
        }
        .unwrap();

        let object_to_wrap = match self.get(object_id, object_type) {
            Some(o) => o,
            None => bail!("no such {:?} object: {:?}", object_type, object_id),
        };

        if !object_to_wrap
            .object_info
            .capabilities
            .contains(Capability::EXPORTABLE_UNDER_WRAP)
        {
            bail!(
                "object {:?} of type {:?} does not have EXPORT_UNDER_WRAP capability",
                object_id,
                object_type
            );
        }

        let mut object_info = object_to_wrap.object_info.clone();

        match object_info.origin {
            ObjectOrigin::Generated => object_info.origin = ObjectOrigin::WrappedGenerated,
            ObjectOrigin::Imported => object_info.origin = ObjectOrigin::WrappedImported,
            ObjectOrigin::WrappedGenerated | ObjectOrigin::WrappedImported => (),
        }

        let mut wrapped_object = serialize(&WrappedObject {
            object_info,
            data: object_to_wrap.payload.as_ref().into(),
        })
        .unwrap();

        // Make room for the MAC
        wrapped_object.extend_from_slice(&[0u8; WRAPPED_DATA_MAC_SIZE]);

        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&wrap_nonce.as_ref()[..12]);

        aead::seal_in_place(
            &sealing_key,
            Nonce::assume_unique_for_key(nonce),
            Aad::from(b""),
            &mut wrapped_object,
            WRAPPED_DATA_MAC_SIZE,
        )
        .unwrap();

        Ok(wrapped_object)
    }

    /// Deserialize an encrypted object and insert it into the HSM
    pub fn unwrap<V: Into<Vec<u8>>>(
        &mut self,
        wrap_key_id: ObjectId,
        wrap_nonce: &WrapNonce,
        ciphertext: V,
    ) -> Result<ObjectHandle, Error> {
        let opening_key = match self.get(wrap_key_id, ObjectType::WrapKey) {
            Some(k) => match k.algorithm().wrap().unwrap() {
                WrapAlg::AES128_CCM => OpeningKey::new(&AES_128_GCM, k.payload.as_ref()),
                WrapAlg::AES256_CCM => OpeningKey::new(&AES_256_GCM, k.payload.as_ref()),
                unsupported => bail!("unsupported wrap key algorithm: {:?}", unsupported),
            }
            .unwrap(),
            None => bail!("no such wrap key: {:?}", wrap_key_id),
        };

        let mut wrapped_data: Vec<u8> = ciphertext.into();

        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&wrap_nonce.as_ref()[..12]);

        if aead::open_in_place(
            &opening_key,
            Nonce::assume_unique_for_key(nonce),
            Aad::from(b""),
            0,
            &mut wrapped_data,
        )
        .is_err()
        {
            bail!("error decrypting wrapped object!");
        }

        let plaintext_len: usize = wrapped_data
            .len()
            .checked_sub(WRAPPED_DATA_MAC_SIZE)
            .unwrap();

        let unwrapped_object: WrappedObject = deserialize(&wrapped_data[..plaintext_len]).unwrap();

        let payload = Payload::new(
            unwrapped_object.object_info.algorithm,
            &unwrapped_object.data,
        );

        let object_key = ObjectHandle::new(
            unwrapped_object.object_info.object_id,
            unwrapped_object.object_info.object_type,
        );

        let object = Object {
            object_info: unwrapped_object.object_info,
            payload,
        };

        assert!(self.0.insert(object_key.clone(), object).is_none());

        Ok(object_key)
    }

    /// Iterate over the objects
    pub fn iter(&self) -> Iter {
        self.0.iter()
    }
}

/// Iterator over objects
pub(crate) type Iter<'a> = BTreeMapIter<'a, ObjectHandle, Object>;