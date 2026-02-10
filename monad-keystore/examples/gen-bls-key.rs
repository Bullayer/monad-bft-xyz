use std::path::Path;
use rand::{rngs::OsRng, RngCore};

use monad_keystore::keystore::{Keystore, KeystoreSecret, KeystoreVersion};

// pub mod keystore;

fn main() -> Result<(), String> {
    println!("It is recommended to generate key in air-gapped machine to be secure.");
    let mut ikm = vec![0_u8; 32];

    OsRng.fill_bytes(&mut ikm);

    println!("Keep your IKM secure: {}", hex::encode(&ikm));

    let keystore_secret = KeystoreSecret::new(ikm);

    let key_type = Some("bls");
    let keystore_path = Path::new("./id-bls");

    if let Some("bls") = key_type {
        let bls_keypair = keystore_secret
        .clone()
        .to_bls(KeystoreVersion::DirectIkm)
        .map_err(|e| format!("failed to create bls keypari {:?}", e))?;
        let private_key = bls_keypair.privkey_view();
        let public_key = bls_keypair.pubkey();
        println!("BLS private key: {}", private_key);
        println!("BLS public key: {:?}", public_key);
    }else{
        let secp_keypair = keystore_secret
        .clone()
        .to_secp(KeystoreVersion::DirectIkm)
        .map_err(|e| format!("failed to create secp keypair {:?}", e))?;
        let private_key = secp_keypair.privkey_view();
        let public_key = secp_keypair.pubkey();
        println!("Secp private key: {}", private_key);
        println!("Secp public key: {:?}", public_key);
    }

    Keystore::create_keystore_json_with_version(
        keystore_secret.as_ref(),
        "",
        &keystore_path,
        KeystoreVersion::DirectIkm
    )
    .map_err(|e| format!("failed to create keystore json {:?}", e))?;

    println!("Successfully created keystore file.");

    Ok(())
}