//! Elliptic Curve Digital Signature Algorithm (ECDSA) tests

use ::ecdsa::{
    elliptic_curve::{
        consts::U1,
        generic_array::ArrayLength,
        sec1::{UncompressedPointSize, UntaggedPointSize},
        weierstrass::{point, Curve},
    },
    signature::Verifier,
};
use std::ops::Add;
use yubihsm::{
    asymmetric::signature::Signer as _,
    ecdsa::{self, algorithm::CurveAlgorithm, NistP256},
    object, Client,
};

#[cfg(feature = "secp256k1")]
use yubihsm::ecdsa::Secp256k1;

/// Domain IDs for test key
const TEST_SIGNING_KEY_DOMAINS: yubihsm::Domain = yubihsm::Domain::DOM1;

/// Capability for test key
const TEST_SIGNING_KEY_CAPABILITIES: yubihsm::Capability = yubihsm::Capability::SIGN_ECDSA;

/// Label for test key
const TEST_SIGNING_KEY_LABEL: &str = "Signatory test key";

/// Example message to sign
const TEST_MESSAGE: &[u8] =
    b"The Elliptic Curve Digital Signature Algorithm (ECDSA) is a variant of the \
      Digital Signature Algorithm (DSA) which uses elliptic curve cryptography.";

/// Create the signer for this test
fn create_signer<C>(key_id: object::Id) -> ecdsa::Signer<C>
where
    C: Curve + CurveAlgorithm + point::Compression,
    UntaggedPointSize<C>: Add<U1> + ArrayLength<u8>,
    UncompressedPointSize<C>: ArrayLength<u8>,
{
    let client = crate::get_hsm_client();
    create_yubihsm_key(&client, key_id, C::asymmetric_algorithm());
    ecdsa::Signer::create(client.clone(), key_id).unwrap()
}

/// Create the key on the YubiHSM to use for this test
fn create_yubihsm_key(client: &Client, key_id: object::Id, alg: yubihsm::asymmetric::Algorithm) {
    // Delete the key in TEST_KEY_ID slot it exists
    // Ignore errors since the object may not exist yet
    let _ = client.delete_object(key_id, yubihsm::object::Type::AsymmetricKey);

    // Create a new key for testing
    client
        .generate_asymmetric_key(
            key_id,
            TEST_SIGNING_KEY_LABEL.into(),
            TEST_SIGNING_KEY_DOMAINS,
            TEST_SIGNING_KEY_CAPABILITIES,
            alg,
        )
        .unwrap();
}

#[test]
fn ecdsa_nistp256_sign_test() {
    let signer = create_signer::<NistP256>(201);
    let signature = signer.sign(TEST_MESSAGE);
    let verifier = p256::ecdsa::VerifyKey::from_encoded_point(&signer.public_key()).unwrap();
    assert!(verifier.verify(TEST_MESSAGE, &signature).is_ok());
}

#[cfg(feature = "secp256k1")]
#[test]
fn ecdsa_secp256k1_sign_test() {
    let signer = create_signer::<Secp256k1>(203);
    let signature = signer.sign(TEST_MESSAGE);
    let verifier = k256::ecdsa::VerifyKey::from_encoded_point(&signer.public_key()).unwrap();
    assert!(verifier.verify(TEST_MESSAGE, &signature).is_ok());
}
